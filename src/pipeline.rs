// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::{anyhow, Context as _};
use futures::future::{Future, FutureExt, TryFutureExt};
use futures::stream::{FuturesUnordered, Stream, StreamExt, TryStream};
use futures::sink::Sink;
use http::Response;
use rumqttc::{ConnectReturnCode, Event, Packet};
use tokio::task::JoinError;
use tracing::{debug, debug_span, error, info, info_span, trace_span};
use tracing_futures::Instrument;
use warp::Filter;

use std::convert::TryInto;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::vec::Vec;

use crate::camera::Camera;
use crate::image_buffer::{BytesImage, ThermalImage};
use crate::occupancy::Tracker;
use crate::render::Renderer as _;
use crate::settings::{RenderSettings, Settings, StreamSettings, TrackerSettings};
use crate::{mqtt, render, spmc, stream};

fn ok_stream<T, St, E>(in_stream: St) -> impl TryStream<Ok = T, Error = E, Item = Result<T, E>>
where
    St: Stream<Item = T>,
{
    in_stream.map(Result::<T, E>::Ok)
}

type InnerTask = Box<dyn Future<Output = anyhow::Result<()>> + Unpin>;
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

pub struct Pipeline {
    camera: Camera,
    frame_source: spmc::Sender<ThermalImage>,
    rendered_source: spmc::Sender<BytesImage>,
    mqtt_client: Option<mqtt::MqttClient>,
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
        let mqtt_client = if let Some(mqtt_config) = config.mqtt {
            debug!("Opening connection to MQTT broker");
            let (client, mut eventloop) = mqtt::MqttClient::connect(mqtt_config)?;
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
            let loop_task: InnerTask = Box::new(
                tokio::spawn(
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
                .map(flatten_join_result),
            );
            tasks.push(loop_task);
            Some(client)
        } else {
            None
        };
        let mut app = Self {
            camera,
            frame_source,
            rendered_source,
            mqtt_client,
            tasks,
        };
        app.create_streams(config.streams)?;
        app.create_tracker(config.tracker)?;
        app.connect_mqtt().await?;
        Ok(app)
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
            self.tasks.push(Box::new(
                tokio::spawn(mjpeg.instrument(trace_span!("mjpeg_encoder"))).err_into(),
            ));
        }
        let combined_route = routes
            .into_iter()
            .reduce(|combined, next| combined.or(next).unify().boxed())
            .ok_or_else(|| anyhow!("problem creating streaming routes"))?;
        let bind_address: std::net::SocketAddr = settings.into();
        debug!(address = ?bind_address, "creating warp server");
        let server = warp::serve(combined_route).bind(bind_address);
        self.tasks.push(Box::new(
            server.instrument(info_span!("warp_server")).map(Ok),
        ));
        Ok(())
    }

    fn create_tracker(&mut self, settings: TrackerSettings) -> anyhow::Result<()> {
        let tracker = Tracker::from(&settings);
        let logged_count_stream = tracker
            .count_stream()
            .instrument(info_span!("occupancy_count_stream"))
            .inspect(|count| {
                info!(occupancy_count = count, "occupancy count changed");
            });
        let frame_stream = self.frame_source.stream();
        self.tasks.push(Box::new(
            ok_stream(logged_count_stream)
                .forward(futures::sink::drain())
                .err_into(),
        ));
        self.tasks.push(Box::new(
            ok_stream(frame_stream).forward(tracker).err_into(),
        ));
        Ok(())
    }

    async fn connect_mqtt(&mut self) -> anyhow::Result<()> {
        // Bail out early if there's no MQTT config
        if let Some(mqtt_client) = &mut self.mqtt_client {
            mqtt_client.publish_initial().await?;
        }
        Ok(())
    }
}

impl Future for Pipeline {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.tasks.poll_next_unpin(cx) {
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
    Ok((frame_multiplexer, Box::new(frame_future)))
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
    let task = Box::new(tokio::spawn(render_future).map(flatten_join_result));
    Ok((rendered_multiplexer, task))
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
