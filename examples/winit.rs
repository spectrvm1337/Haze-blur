//! `Haze::attach` on a winit window. Drag it with the left button, Esc closes.
//! `--seconds 3` closes the window by itself after 3 s.
use std::time::{Duration, Instant};

use haze::{Haze, Style};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Window, WindowId};

struct App {
    window: Option<Window>,
    haze: Option<Haze>,
    started: Instant,
    last_change: Instant,
    light: bool,
    close_after: Option<Duration>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = Window::default_attributes()
            .with_title("Haze on winit")
            .with_inner_size(LogicalSize::new(520.0, 340.0))
            .with_decorations(false)
            .with_transparent(true)
            // Without this the window keeps a surface of its own, and it shows white outside the rounded corners.
            .with_no_redirection_bitmap(true);
        let window = event_loop.create_window(attributes).expect("window");
        let haze = Haze::attach(&window, Style::dark().radius(32.0)).expect("attach");
        println!("attached to a winit window");
        self.window = Some(window);
        self.haze = Some(haze);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let Some(w) = &self.window {
                    let _ = w.drag_window();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.logical_key == Key::Named(NamedKey::Escape) => {
                event_loop.exit();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.close_after.is_some_and(|d| self.started.elapsed() >= d) {
            event_loop.exit();
            return;
        }
        if self.last_change.elapsed() >= Duration::from_secs(2) {
            self.last_change = Instant::now();
            self.light = !self.light;
            let next = if self.light { Style::frost().radius(64.0) } else { Style::dark().radius(20.0) };
            if let Some(h) = &self.haze {
                h.animate_to(next, Duration::from_millis(900));
            }
        }
        event_loop.set_control_flow(ControlFlow::wait_duration(Duration::from_millis(100)));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let close_after =
        args.iter().position(|a| a == "--seconds").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).map(Duration::from_secs);
    let now = Instant::now();
    let mut app = App { window: None, haze: None, started: now, last_change: now, light: false, close_after };
    EventLoop::new().expect("event loop").run_app(&mut app).expect("run");
}
