// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::{anyhow, Context as _};
use futures::future::{Future, FutureExt, TryFutureExt};
use futures::sink::Sink;
use futures::stream::{FuturesUnordered, Stream, StreamExt, TryStream};
use http::Response;
use pin_project::pin_project;
use rumqttc::{
    AsyncClient, ConnectReturnCode, Event, LastWill, MqttOptions as RuMqttOptions, Packet, QoS,
};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinError;
use tracing::{debug, debug_span, error, info, info_span, trace_span, warn};
use tracing_futures::Instrument;
use warp::Filter;

use std::convert::{TryFrom, TryInto};
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::vec::Vec;

use crate::camera::Camera;
use crate::image_buffer::{BytesImage, ThermalImage};
use crate::mqtt::{home_assistant as hass, MqttSettings, Occupancy, OccupancyCount, State, Status};
use crate::occupancy::Tracker;
use crate::render::Renderer as _;
use crate::settings::{RenderSettings, Settings, StreamSettings, TrackerSettings};
use crate::{render, spmc, stream};

const MQTT_BASE_TOPIC: &str = "r-u-still-there";

fn ok_stream<T, St, E>(in_stream: St) -> impl TryStream<Ok = T, Error = E, Item = Result<T, E>>
where
    St: Stream<Item = T>,
{
    in_stream.map(Result::<T, E>::Ok)
}

type MqttClient = Arc<AsyncMutex<AsyncClient>>;
type ArcDevice = Arc<hass::Device>;
type InnerTask = Pin<Box<dyn Future<Output = anyhow::Result<()>>>>;
type TaskList = FuturesUnordered<InnerTask>;

fn flatten_join_result<E>(join_result: Result<Result<(), E>, JoinError>) -> anyhow::Result<()>
where
    anyhow::Error: From<E>,
{
    match join_result {
        Ok(inner_result) => Ok(inner_result?),
        Err(join_error) => {
            if join_error.is_panic() {
                join_error.into_panic();
                unreachable!()
            } else {
                Err(join_error.into())
            }
        }
    }
}

#[pin_project]
pub struct Pipeline {
    camera: Camera,
    frame_source: spmc::Sender<ThermalImage>,
    rendered_source: spmc::Sender<BytesImage>,
    mqtt_client: MqttClient,
    status: State<Status, ArcDevice>,
    hass_device: ArcDevice,
    // Keep the MQTT config around as we might need to use it when reconnecting.
    mqtt_config: MqttSettings,
    #[pin]
    tasks: TaskList,
}

impl Pipeline {
    pub async fn new(config: Settings) -> anyhow::Result<Self> {
        let camera_settings = &config.camera;
        let camera: Camera = camera_settings.try_into()?;
        let (frame_source, frame_task) = create_frame_source(&camera)?;
        let (rendered_source, render_task) = create_renderer(&frame_source, config.render)?;
        // Once IntoIterator is implemented for arrays, this line can be simplified
        let tasks: TaskList = std::array::IntoIter::new([frame_task, render_task]).collect();
        debug!("Opening connection to MQTT broker");
        let (mqtt_client, loop_task, status) = connect_mqtt(&config.mqtt).await?;
        tasks.push(loop_task);
        // Create a device for HAss integration. It's still used even if the HAss messages aren;t
        // being sent.
        let hass_device = Self::create_device(&config.mqtt.name, config.mqtt.unique_id());
        let mut app = Self {
            camera,
            frame_source,
            rendered_source,
            mqtt_client,
            status,
            hass_device,
            mqtt_config: config.mqtt,
            tasks,
        };
        app.create_streams(config.streams)?;
        app.create_tracker(config.tracker).await?;
        Ok(app)
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

    fn create_streams(&mut self, settings: StreamSettings) -> anyhow::Result<()> {
        // Bail out if there aren't any stream sources enabled.
        // For now there's just MJPEG, but HLS is planned for the future.
        if !settings.mjpeg {
            info!("video streams disabled, skipping setup");
            // It's Ok, there was just nothing to do.
            return Ok(());
        }
        let mut routes = Vec::new();
        if settings.mjpeg {
            debug!("creating MJPEG encoder");
            // MJPEG sink
            let mjpeg = stream::mjpeg::MjpegStream::new(&self.rendered_source);
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
            // Stream out rendered frames via MJPEG
            self.tasks.push(
                tokio::spawn(mjpeg.instrument(trace_span!("mjpeg_encoder")))
                    .err_into()
                    .boxed(),
            );
        }
        let combined_route = routes
            .into_iter()
            .reduce(|combined, next| combined.or(next).unify().boxed())
            .ok_or_else(|| anyhow!("problem creating streaming routes"))?;
        let bind_address: std::net::SocketAddr = settings.into();
        debug!(address = ?bind_address, "creating warp server");
        let server = warp::serve(combined_route).bind(bind_address);
        self.tasks
            .push(server.instrument(info_span!("warp_server")).map(Ok).boxed());
        Ok(())
    }

    async fn create_tracker(&mut self, settings: TrackerSettings) -> anyhow::Result<()> {
        let tracker = Tracker::from(&settings);
        let count = State::new_discoverable(
            Arc::clone(&self.mqtt_client),
            Arc::clone(&self.hass_device),
            &MQTT_BASE_TOPIC,
            "count",
            true,
            QoS::AtLeastOnce,
        );
        let occupied = State::new_discoverable(
            Arc::clone(&self.mqtt_client),
            Arc::clone(&self.hass_device),
            &MQTT_BASE_TOPIC,
            "occupied",
            true,
            QoS::AtLeastOnce,
        );
        if self.mqtt_config.home_assistant {
            count
                .publish_home_assistant_discovery(
                    &self.mqtt_config.home_assistant_topic,
                    self.status.topic(),
                )
                .await?;
            occupied
                .publish_home_assistant_discovery(
                    &self.mqtt_config.home_assistant_topic,
                    self.status.topic(),
                )
                .await?;
        }
        let count_sink = count.sink();
        let update_count_stream = ok_stream(tracker.count_stream().map(OccupancyCount::from))
            //.instrument(info_span!("occupancy_count_stream"))
            .forward(count_sink)
            .boxed();
        self.tasks.push(update_count_stream);
        let occupied_sink = occupied.sink();
        let update_occupied_stream = ok_stream(tracker.count_stream().map(Occupancy::from))
            //.instrument(info_span!("occupancy_count_stream"))
            .forward(occupied_sink)
            .boxed();
        self.tasks.push(update_occupied_stream);
        let frame_stream = self.frame_source.stream();
        self.tasks
            .push(ok_stream(frame_stream).forward(tracker).err_into().boxed());
        Ok(())
    }
}

impl Future for Pipeline {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.as_mut().project().tasks.poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(option) => match option {
                    None => return Poll::Ready(()),
                    Some(res) => {
                        if let Err(err) = res {
                            error!(error = ?err, "error in task");
                        }
                    }
                },
            }
        }
    }
}

fn create_frame_source(camera: &Camera) -> anyhow::Result<(spmc::Sender<ThermalImage>, InnerTask)> {
    let frame_stream = camera.frame_stream();
    let frame_multiplexer = spmc::Sender::default();
    let frame_future = frame_stream.forward(frame_multiplexer.clone());
    Ok((frame_multiplexer, frame_future.boxed()))
}

fn create_renderer(
    frame_source: &spmc::Sender<ThermalImage>,
    settings: RenderSettings,
) -> anyhow::Result<(spmc::Sender<BytesImage>, InnerTask)> {
    let renderer = render::SvgRenderer::new(
        settings.lower_limit,
        settings.upper_limit,
        render::TemperatureDisplay::from(settings.units),
        settings.grid_size,
        settings.colors,
    );
    let rendered_stream = frame_source
        .stream()
        .instrument(trace_span!("render_stream"))
        .map(move |temperatures| renderer.render_buffer(&temperatures));
    let rendered_multiplexer = spmc::Sender::default();
    let render_future = ok_stream(rendered_stream).forward(rendered_multiplexer.clone());
    let task = tokio::spawn(render_future).map(flatten_join_result).boxed();
    Ok((rendered_multiplexer, task))
}

const EVENT_LOOP_CAPACITY: usize = 20;

async fn connect_mqtt(
    settings: &MqttSettings,
) -> anyhow::Result<(MqttClient, InnerTask, State<Status, ArcDevice>)> {
    let base_topic = "r-u-still-there";
    let status_topic = [MQTT_BASE_TOPIC, &settings.name, "status"].join("/");
    let mut client_options = RuMqttOptions::try_from(settings)?;
    client_options.set_last_will(LastWill::new(
        &status_topic,
        Status::Offline.to_string().as_bytes(),
        QoS::AtLeastOnce,
        true,
    ));
    let (client, mut eventloop) = AsyncClient::new(client_options, EVENT_LOOP_CAPACITY);
    let client = Arc::new(AsyncMutex::new(client));
    // Wait until we get a ConnAck packet from the broker before continuing with setup.
    loop {
        let event = eventloop.poll().await?;
        if let Event::Incoming(Packet::ConnAck(conn_ack)) = event {
            if conn_ack.code == ConnectReturnCode::Success {
                debug!("Connected to MQTT broker");
                break;
            } else {
                error!(response_code = ?conn_ack.code, "Connection to MQTT broker refused.");
                return Err(anyhow!("Connection to MQTT broker refused"));
            }
        }
    }
    // Create a status State for use during setup.
    let status = State::new(
        Arc::clone(&client),
        base_topic,
        &settings.name,
        "status",
        true,
        QoS::AtLeastOnce,
    );
    // This won't get actually sent to the broker until the loop task starts getting run (see
    // below). Instead it gets added to the queue of messages to be sent.
    status.publish().await?;
    // Create a task to run the event loop
    let loop_task: InnerTask = tokio::spawn(
        async move {
            loop {
                match eventloop.poll().await.context("polling MQTT event loop") {
                    Ok(event) => debug!(?event, "MQTT event processed"),
                    Err(err) => {
                        error!(error = ?err, "Error with MQTT connection");
                        return Err(err);
                    }
                }
            }
            // This weird looking bit is a back-door type annotation for the return type of the
            // async closure. It's unreachable, but necessary (for now).
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        }
        .instrument(debug_span!("mqtt_event_loop")),
    )
    .map(flatten_join_result)
    .boxed();
    Ok((client, loop_task, status))
}

/// A drain with a generic error.
///
/// [futures::sink::drain] has [std::convert::Infallible] as its `Error` type, which precludes it
/// being used with other types of errors, which can be desired when
/// [forwarding][futures::stream::StreamExt::forward] a [Stream][futures::stream::Stream] to a
/// [Sink][futures::sink::Sink].
struct Drain<E>(PhantomData<E>);

impl<E> Drain<E> {
    fn new() -> Self {
        Drain(PhantomData)
    }
}

impl<E> Sink<()> for Drain<E> {
    type Error = E;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, _: ()) -> Result<(), Self::Error> {
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}
