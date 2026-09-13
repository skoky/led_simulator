//! A small always-on-top window mirroring the current LED color.
//!
//! winit wants its event loop on the main thread, so when the popup is enabled the CUSE
//! session moves to a worker thread and egui keeps the main one.

use crate::display;
use crate::ws2812::Rgb;
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Height reserved for the caption under the swatch.
const CAPTION_HEIGHT: f32 = 46.0;

/// Shared with the device side, which pushes every color change into it.
#[derive(Clone, Default)]
pub struct Handle {
    pixels: Arc<Mutex<Vec<Rgb>>>,
}

impl Handle {
    pub fn set(&self, pixels: &[Rgb]) {
        *self.pixels.lock().unwrap_or_else(|e| e.into_inner()) = pixels.to_vec();
    }

    fn get(&self) -> Vec<Rgb> {
        self.pixels.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

struct Popup {
    handle: Handle,
    /// Set when the CUSE session ends, so the window closes with it.
    stop: Arc<AtomicBool>,
    /// Whether the always-on-top request has been sent (see [`Self::stay_on_top`]).
    raised: bool,
}

impl eframe::App for Popup {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.stop.load(Ordering::Relaxed) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        self.stay_on_top(ui.ctx());

        let pixels = self.handle.get();
        let full = ui.available_rect_before_wrap();
        let split = (full.max.y - CAPTION_HEIGHT).max(full.min.y);

        swatch(ui, egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, split)), &pixels);

        let caption_rect = egui::Rect::from_min_max(egui::pos2(full.min.x, split), full.max);
        let mut caption_ui = ui.new_child(egui::UiBuilder::new().max_rect(caption_rect));
        caption(&mut caption_ui, &pixels);

        // Colors arrive on the CUSE thread; polling beats plumbing a repaint signal
        // through the callbacks, and 10 Hz is well past what an eye follows.
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
}

impl Popup {
    /// Asks for always-on-top once the window exists. The same wish is in the viewport
    /// builder, but window managers are more reliable about honouring the request made
    /// after the window is mapped.
    fn stay_on_top(&mut self, ctx: &egui::Context) {
        if self.raised {
            return;
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
        self.raised = true;
    }
}

/// One filled rectangle per LED, side by side.
fn swatch(ui: &egui::Ui, rect: egui::Rect, pixels: &[Rgb]) {
    let painter = ui.painter();
    let outline = egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);

    if pixels.is_empty() {
        painter.rect_stroke(rect.shrink(1.0), 4.0, outline, egui::StrokeKind::Inside);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "waiting for frames",
            egui::FontId::proportional(13.0),
            ui.visuals().weak_text_color(),
        );
        return;
    }

    let width = rect.width() / pixels.len() as f32;

    for (i, px) in pixels.iter().enumerate() {
        let cell = egui::Rect::from_min_size(
            egui::pos2(rect.min.x + width * i as f32, rect.min.y),
            egui::vec2(width, rect.height()),
        )
        .shrink(1.0);

        painter.rect_filled(cell, 4.0, egui::Color32::from_rgb(px.r, px.g, px.b));
        // Keeps a black LED visible against a dark background.
        painter.rect_stroke(cell, 4.0, outline, egui::StrokeKind::Inside);
    }
}

fn caption(ui: &mut egui::Ui, pixels: &[Rgb]) {
    ui.add_space(4.0);

    match pixels {
        [] => {
            ui.label("no data yet");
        }
        [one] => {
            ui.horizontal(|ui| {
                ui.monospace(display::hex(*one));
                ui.label(display::describe(*one));
            });
            ui.label(format!("rgb({}, {}, {})", one.r, one.g, one.b));
        }
        many => {
            ui.label(format!("{} LEDs", many.len()));
            ui.monospace(many.iter().map(|px| display::hex(*px)).collect::<Vec<_>>().join(" "));
        }
    }
}

/// Runs the window. Returns when it is closed, or when `stop` is set.
pub fn run(handle: Handle, stop: Arc<AtomicBool>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LED simulator")
            .with_inner_size([260.0, 160.0])
            .with_min_inner_size([160.0, 110.0])
            .with_always_on_top(),
        ..Default::default()
    };

    eframe::run_native(
        "led_simulator",
        options,
        Box::new(|_cc| {
            Ok(Box::new(Popup {
                handle,
                stop,
                raised: false,
            }))
        }),
    )
}
