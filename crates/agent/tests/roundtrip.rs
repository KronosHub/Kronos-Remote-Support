//! App ↔ agent over an in-memory pipe, with a test-pattern capture and a recording injector.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent::client::{AgentCapturer, AgentInput};
use agent::server::{Backend, Injector, serve};
use capture::{CaptureConfig, Capturer, TestPattern};
use protocol::InputEvent;

#[derive(Default)]
struct Recorder {
    events: Arc<Mutex<Vec<String>>>,
}

struct RecordingInjector(Arc<Mutex<Vec<String>>>);

impl Injector for RecordingInjector {
    fn send(&self, event: InputEvent) {
        self.0.lock().unwrap().push(format!("{event:?}"));
    }
    fn set_area(&self, area: input::Area) {
        self.0
            .lock()
            .unwrap()
            .push(format!("area {}x{}", area.width, area.height));
    }
}

impl Backend for Recorder {
    fn start_capture(&self, config: &CaptureConfig) -> capture::Result<Box<dyn Capturer>> {
        let (w, h) = config.max_size.unwrap();
        Ok(Box::new(TestPattern::new(w, h, config.max_fps)))
    }
    fn start_input(&self) -> anyhow::Result<Box<dyn Injector>> {
        Ok(Box::new(RecordingInjector(self.events.clone())))
    }
}

fn pipe(backend: Arc<dyn Backend>, rt: &tokio::runtime::Runtime) -> tokio::io::DuplexStream {
    let (app, agent) = tokio::io::duplex(1 << 20);
    rt.spawn(serve(agent, backend));
    app
}

#[test]
fn capture_and_input_through_the_agent() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let recorder = Recorder::default();
    let events = recorder.events.clone();
    let backend: Arc<dyn Backend> = Arc::new(recorder);

    let config = CaptureConfig {
        display_id: None,
        max_fps: 30,
        max_size: Some((320, 180)),
        show_cursor: true,
        native: false,
    };
    let app_end = pipe(backend.clone(), &rt);
    let mut capturer = AgentCapturer::start(move || async move { Ok(app_end) }, &config).unwrap();
    assert_eq!(capturer.output_size(), (320, 180));
    let frames = (0..30)
        .filter_map(|_| capturer.next_frame(Duration::from_millis(100)).unwrap())
        .take(3)
        .collect::<Vec<_>>();
    assert_eq!(frames.len(), 3);
    assert!(frames.iter().all(|f| f.data.len() == 320 * 180 * 4));

    let app_end = pipe(backend, &rt);
    let input = AgentInput::start(move || async move { Ok(app_end) }).unwrap();
    input.set_area(input::Area {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    });
    input.send(InputEvent::MouseMove { x: 0.5, y: 0.25 });
    std::thread::sleep(Duration::from_millis(200));
    let events = events.lock().unwrap().clone();
    assert_eq!(events, ["area 1920x1080", "MouseMove { x: 0.5, y: 0.25 }"]);
    assert_eq!(input.sent(), 1);
}

#[test]
fn capture_errors_reach_the_app() {
    struct Broken;
    impl Backend for Broken {
        fn start_capture(&self, _: &CaptureConfig) -> capture::Result<Box<dyn Capturer>> {
            Err(capture::CaptureError::PermissionDenied("nope".into()))
        }
        fn start_input(&self) -> anyhow::Result<Box<dyn Injector>> {
            anyhow::bail!("no input")
        }
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    let app_end = pipe(Arc::new(Broken), &rt);
    let err = AgentCapturer::start(
        move || async move { Ok(app_end) },
        &CaptureConfig::default(),
    )
    .err()
    .expect("capture refused");
    assert!(err.to_string().contains("nope"), "{err}");
    let app_end = pipe(Arc::new(Broken), &rt);
    let err = AgentInput::start(move || async move { Ok(app_end) })
        .err()
        .unwrap();
    assert!(err.to_string().contains("no input"));
}
