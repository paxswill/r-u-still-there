// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::{anyhow, Context as _};
use futures::future::{Future, FutureExt, TryFutureExt};
use futures::ready;
use futures::stream::{BoxStream, FuturesUnordered, Stream, StreamExt};
use http::Response;
use pin_project::pin_project;
use rumqttc::QoS;
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio::task::spawn_blocking;
use tokio::time::Duration;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};
use tracing::{debug, debug_span, error, info, info_span, trace, trace_span, warn};
use tracing_futures::Instrument;
use warp::Filter;

use std::convert::{TryFrom, TryInto};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{mpsc, Arc};
use std::task::{Context, Poll};

use crate::camera::{Camera, CameraCommand, Measurement};
use crate::image_buffer::BytesImage;
use crate::mqtt::{
    home_assistant as hass, MqttClient, MqttSender, MqttSettings, Occupancy, OccupancyCount, State,
};
use crate::occupancy::{Tracker, TrackerSettings};
use crate::settings::Settings;
use crate::util::{flatten_join_result, StreamExt as _};
use crate::{render, spmc, stream};

type ArcDevice = Arc<hass::Device>;
type InnerTask = Pin<Box<dyn Future<Output = anyhow::Result<()>>>>;
type TaskList = FuturesUnordered<InnerTask>;
type MeasurementStream<'a> = BoxStream<'a, Measurement>;

#[pin_project]
pub(crate) struct Pipeline {
    camera_command_channel: mpsc::Sender<CameraCommand>,
    rendered_source: spmc::Sender<BytesImage>,
    mqtt_sender: MqttSender,
    mqtt_config: MqttSettings,
    status_topic: String,
    hass_device: ArcDevice,
    #[pin]
    tasks: TaskList,
}

impl Pipeline {
    pub(crate) async fn new(config: Settings) -> anyhow::Result<Self> {
        let camera_settings = &config.camera;
        let camera: Camera = camera_settings
            .try_into()
            .context("Error configuring camera")?;
        let camera_command_channel = camera.command_channel();
        let camera_task = spawn_blocking(move || {
            camera
                .measurement_loop()
                .context("Error within camera frame thread")
        })
        .map(flatten_join_result)
        .boxed();
        let frame_rate_limit = config.streams.common_frame_rate();
        let measurement_stream = Self::create_measurement_stream(&camera_command_channel)
            .await
            .context("Error requesting measurement stream from camera")?;
        let (rendered_source, render_task) =
            create_renderer(measurement_stream, config.render, frame_rate_limit)?;
        let mqtt_client = MqttClient::new(&config.mqtt)?;
        let mqtt_sender = mqtt_client.new_sender();
        let status_topic = mqtt_client.status_topic().to_string();
        let mqtt_client = tokio::spawn(mqtt_client.run_loop())
            .map(flatten_join_result)
            .boxed();
        // Once IntoIterator is implemented for arrays, this line can be simplified
        let tasks: TaskList =
            std::array::IntoIter::new([render_task, camera_task, mqtt_client]).collect();
        debug!("Opening connection to MQTT broker");
        // Create a device for HAss integration. It's still used even if the HAss messages aren;t
        // being sent.
        let hass_device = Self::create_device(&config.mqtt.name, config.mqtt.unique_id());
        let mut app = Self {
            camera_command_channel,
            rendered_source,
            mqtt_sender,
            mqtt_config: config.mqtt,
            status_topic,
            hass_device,
            tasks,
        };
        app.record_measurements(
            config
                .camera
                .extra()
                .get("path")
                .and_then(toml::Value::as_str)
                .map(PathBuf::from),
        )
        .await
        .context("Error configuring camera frame recording")?;
        app.create_streams(config.streams)
            .context("Error creating video streams")?;
        app.create_tracker(config.tracker)
            .await
            .context("Error creating occupancy tracker")?;
        app.create_thermometer()
            .await
            .context("Error creating ambient temperature monitor")?;
        Ok(app)
    }

    // Get a Stream of Measurements from the camera.
    async fn create_measurement_stream(
        command_channel: &mpsc::Sender<CameraCommand>,
    ) -> anyhow::Result<MeasurementStream<'static>> {
        let (command_tx, command_rx) = oneshot::channel();
        command_channel.send(CameraCommand::Subscribe(command_tx))?;
        let new_subscription = command_rx.await.context("Creating subscription stream")?;
        let measurement_stream =
            BroadcastStream::new(new_subscription).filter_map(|broadcast_res| async move {
                match broadcast_res {
                    Ok(measurement) => Some(measurement),
                    Err(BroadcastStreamRecvError::Lagged(lag_count)) => {
                        warn!("Measurement sink lagging {} samples", lag_count);
                        None
                    }
                }
            });
        Ok(measurement_stream.boxed())
    }

    fn create_device(device_name: &str, unique_id: String) -> ArcDevice {
        let mut device = hass::Device::default();
        // Add all the MAC addresses to our device, it'll update whatever Home Assistant has.
        let mac_addresses = match mac_address::MacAddressIterator::new() {
            Ok(address_iterator) => Some(address_iterator),
            Err(e) => {
                warn!("unable to access MAC addresses: {:?}", e);
                None
            }
        };
        if let Some(address_iterator) = mac_addresses {
            // Filter out all-zero MAC addresses (like from a loopback interface)
            let filtered_addresses = address_iterator.filter(|a| a.bytes() != [0u8; 6]);
            for address in filtered_addresses {
                device.add_mac_connection(address);
            }
        }
        device.name = Some(device_name.to_string());
        device.add_identifier(unique_id);
        // TODO: investigate using the 'built' crate to also get Git hash
        device.sw_version =
            option_env!("CARGO_PKG_VERSION").map(|vers| format!("r-u-still-there v{}", vers));
        Arc::new(device)
    }

    fn create_streams(&mut self, settings: stream::StreamSettings) -> anyhow::Result<()> {
        // Bail out if there aren't any stream sources enabled.
        // For now there's just MJPEG, but HLS is planned for the future.
        if !settings.any_streams_enabled() {
            info!("video streams disabled, skipping streams setup");
            // It's Ok, there was just nothing to do.
            return Ok(());
        }
        let mut routes = Vec::new();
        if settings.mjpeg.enabled {
            debug!("creating JPEG encoder");
            let jpeg_sender = self.rendered_source.new_child();
            let encoder_stream = self
                .rendered_source
                .uncounted_stream()
                .then(|image| async move {
                    let res = spawn_blocking(move || stream::encode_jpeg(&image))
                        .map(flatten_join_result)
                        .await;
                    // Map the JoinError to an anyhow::Error
                    res.map_err(|err| anyhow!("Error with JPEG encoding thread: {:?}", err))
                });
            // MJPEG sink
            let mjpeg = stream::MjpegStream::new(&jpeg_sender);
            let mjpeg_output = mjpeg.clone();
            let mjpeg_route = warp::path("mjpeg")
                .and(warp::path::end())
                .map(move || {
                    Response::builder()
                        .status(200)
                        .header("Content-Type", mjpeg_output.content_type())
                        .body(mjpeg_output.body())
                })
                .boxed();
            routes.push(mjpeg_route);
            // Forward the encoded JPEG images to the MJPEG "encoder"
            self.tasks.push(encoder_stream.forward(jpeg_sender).boxed());
            // Run the MJPEG task that broadcasts the framed data to connected clients
            self.tasks.push(
                tokio::spawn(mjpeg.instrument(trace_span!("mjpeg_framer")))
                    .err_into()
                    .boxed(),
            );
        }
        if settings.http_streams_enabled() {
            let combined_route = routes
                .into_iter()
                .reduce(|combined, next| combined.or(next).unify().boxed())
                .ok_or_else(|| anyhow!("problem creating streaming routes"))?;
            let bind_address: std::net::SocketAddr = settings.into();
            debug!(address = ?bind_address, "creating warp server");
            let server = warp::serve(combined_route).bind(bind_address);
            self.tasks
                .push(server.instrument(info_span!("warp_server")).map(Ok).boxed());
        }
        Ok(())
    }

    /// Create an occupancy tracker with the given settings and an expected frame duration.
    async fn create_tracker(&mut self, settings: TrackerSettings) -> anyhow::Result<()> {
        let tracker = Tracker::new(&settings);
        let mut count = State::new_discoverable(
            self.mqtt_sender.clone(),
            Arc::clone(&self.hass_device),
            &self.mqtt_config.base_topic,
            "count",
            true,
            QoS::AtLeastOnce,
        );
        let mut occupied = State::new_discoverable(
            self.mqtt_sender.clone(),
            Arc::clone(&self.hass_device),
            &self.mqtt_config.base_topic,
            "occupied",
            true,
            QoS::AtLeastOnce,
        );
        if self.mqtt_config.home_assistant.enabled {
            count
                .publish_home_assistant_discovery::<usize>(
                    &self.mqtt_config.home_assistant.topic,
                    &self.status_topic,
                )
                .await?;
            occupied
                .publish_home_assistant_discovery::<bool>(
                    &self.mqtt_config.home_assistant.topic,
                    &self.status_topic,
                )
                .await?;
        }
        let count_sink = count.sink();
        let update_count_stream = tracker
            .count_stream()
            .map(OccupancyCount::from)
            .filter_repeated()
            .never_error()
            .forward(count_sink)
            .boxed();
        self.tasks.push(update_count_stream);
        let occupied_sink = occupied.sink();
        let update_occupied_stream = tracker
            .count_stream()
            .map(Occupancy::from)
            .filter_repeated()
            .never_error()
            .forward(occupied_sink)
            .boxed();
        self.tasks.push(update_occupied_stream);
        let measurement_stream = Self::create_measurement_stream(&self.camera_command_channel)
            .await?
            .instrument(info_span!("tracker_measurements"));
        self.tasks.push(
            measurement_stream
                .never_error()
                .forward(tracker)
                .err_into()
                .boxed(),
        );
        Ok(())
    }

    async fn create_thermometer(&mut self) -> anyhow::Result<()> {
        info!("Creating thermometer");
        let unit = self.mqtt_config.home_assistant.unit;
        let temperature_stream = Self::create_measurement_stream(&self.camera_command_channel)
            .await?
            .instrument(info_span!("temperature_measurement"))
            .map(move |measurement| measurement.temperature.in_unit(&unit));
        let state = State::new_discoverable(
            self.mqtt_sender.clone(),
            Arc::clone(&self.hass_device),
            &self.mqtt_config.base_topic,
            "temperature",
            true,
            QoS::AtLeastOnce,
        );
        if self.mqtt_config.home_assistant.enabled {
            let mut config = state
                .discovery_config::<f32>(&self.status_topic)
                .ok_or_else(|| {
                    anyhow!("A discoverable state should have a discovery configuration")
                })?;
            config.set_device_class(hass::AnalogSensorClass::Temperature);
            config.set_unit_of_measurement(Some(self.mqtt_config.home_assistant.unit.to_string()));
            let config_topic = state
                .discovery_topic::<f32>(&self.mqtt_config.home_assistant.topic)
                .ok_or_else(|| anyhow!("A discoverable state should have a discovery topic"))?;
            // Keep this message the same as the debug message in mqtt::state::State::publish_home_assistant_discovery
            debug!(?config, "Publishing Home Assistant discovery config");
            self.mqtt_sender
                .enqueue_publish(config_topic, QoS::AtLeastOnce, &config, true)
                .await?;
        }
        let temperature_sink = state.sink();
        self.tasks.push(
            temperature_stream
                .filter_repeated()
                .never_error()
                .forward(temperature_sink)
                .boxed(),
        );
        Ok(())
    }

    // No-op version for when the mock_camera feature isn't enabled.
    #[cfg(not(feature = "mock_camera"))]
    async fn record_measurements(&mut self, _path: Option<PathBuf>) -> anyhow::Result<()> {
        warn!("Mock camera recording path set, but mock camera support has not been enabled.");
        Ok(())
    }

    #[cfg(feature = "mock_camera")]
    async fn record_measurements(&mut self, path: Option<PathBuf>) -> anyhow::Result<()> {
        if let Some(record_path) = path {
            info!(path = ?record_path, "Recording measurement data");
            let file = tokio::fs::File::create(record_path).await?;
            // Should there be a BufWriter in here? I don't think so, as I won't be able to ensure
            // that flush() is called.
            let bincode_sink: async_bincode::AsyncBincodeWriter<
                tokio::fs::File,
                crate::camera::MeasurementData,
                async_bincode::SyncDestination,
            > = file.into();
            let measurement_stream = Self::create_measurement_stream(&self.camera_command_channel)
                .await?
                .instrument(info_span!("mock_recording"))
                .scan(
                    None,
                    |last_frame_time: &mut Option<std::time::Instant>, measurement: Measurement| {
                        // Swap in the measurement time for this frame for the previous time. The first
                        // frame has `None` as the previous time, so it uses 0 for the duration.
                        let previous_instant = last_frame_time.replace(std::time::Instant::now());
                        let frame_delay = previous_instant.map_or(Duration::ZERO, |i| i.elapsed());
                        let timed_measurement =
                            crate::camera::MeasurementData::new(measurement, frame_delay);
                        std::future::ready(Some(timed_measurement))
                    },
                );
            let recording_task = measurement_stream
                .never_error()
                .forward(bincode_sink)
                .err_into()
                .boxed();
            self.tasks.push(recording_task);
            Ok(())
        } else {
            Ok(())
        }
    }
}

impl Future for Pipeline {
    type Output = anyhow::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        Poll::Ready(loop {
            if let Some(res) = ready!(this.tasks.as_mut().poll_next(cx)) {
                debug!(result = ?res, "Pipeline terminating");
                break res;
            }
        })
    }
}

fn create_renderer(
    measurement_stream: MeasurementStream<'static>,
    settings: render::RenderSettings,
    frame_rate_limit: Option<Duration>,
) -> anyhow::Result<(spmc::Sender<BytesImage>, InnerTask)> {
    let renderer = Arc::new(AsyncMutex::new(render::layer::ImageLayers::try_from(
        settings,
    )?));
    let rendered_stream = match frame_rate_limit {
        None => measurement_stream,
        Some(limit) => tokio_stream::StreamExt::throttle(measurement_stream, limit).boxed(),
    }
    .instrument(info_span!("render_stream"))
    .then(move |measurement| {
        let renderer = Arc::clone(&renderer);
        async move {
            let unlocked_renderer = renderer.lock().await;
            unlocked_renderer.render(measurement).await
        }
    });
    let rendered_multiplexer = spmc::Sender::default();
    let task = rendered_stream
        .forward(rendered_multiplexer.clone())
        .boxed();
    Ok((rendered_multiplexer, task))
}
