use crate::ipc::{self, FromPopup, PopupState, PopupView};
use crate::monitor::Device;
use crate::settings::{Settings, THEME_DARK, THEME_LIGHT};
use crate::theme::{self, Theme};
use crate::win::{self, PixelPoint};
use eframe::egui::{
    self, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2, ViewportCommand,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::io::BufReader;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const FIRST_MESSAGE_TIMEOUT: Duration = Duration::from_millis(200);
const FOREGROUND_ATTEMPTS: u8 = 3;
const POPUP_WIDTH: f32 = 480.0;
const HEADER_HEIGHT: f32 = 36.0;
const HEADER_GAP: f32 = 10.0;
const ROW_MINIMUM_HEIGHT: f32 = 60.0;
const ROW_GAP: f32 = 1.0;
const PAD_X: f32 = 13.0;
const PAD_Y: f32 = 11.0;
const MAXIMUM_LIST_HEIGHT: f32 = 300.0;
const SCROLLBAR_ALLOWANCE: f32 = 12.0;
const INFO_LEFT: f32 = 170.0;
const INFO_RIGHT_MARGIN: f32 = 8.0;
const SCREEN_MARGIN: f32 = 8.0;
const ANCHOR_OFFSET_X: f32 = 28.0;
const ANCHOR_OFFSET_Y: f32 = 18.0;
const DEMO_SLACK: f32 = 120.0;
const KEY_SCROLL_STEP: f32 = 48.0;

enum Incoming {
    State(Box<PopupState>),
    Disconnected,
}

type Connection = (Receiver<Incoming>, Arc<OnceLock<egui::Context>>);

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Connected,
    Demo,
}

pub fn run_connected(arguments: &[String]) -> eframe::Result {
    let anchor = arguments
        .iter()
        .position(|argument| argument == "--anchor")
        .and_then(|index| arguments.get(index + 1))
        .and_then(|value| parse_anchor(value));

    let (sender, receiver) = mpsc::channel::<Incoming>();
    let repaint = Arc::new(OnceLock::<egui::Context>::new());
    let reader_repaint = Arc::clone(&repaint);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(std::io::stdin());
        loop {
            let incoming = match ipc::read_message::<PopupState>(&mut reader) {
                Ok(Some(state)) => Incoming::State(Box::new(state)),
                _ => Incoming::Disconnected,
            };
            let disconnected = matches!(incoming, Incoming::Disconnected);
            if sender.send(incoming).is_err() {
                return;
            }
            if let Some(ctx) = reader_repaint.get() {
                ctx.request_repaint();
            }
            if disconnected {
                return;
            }
        }
    });

    let first = match receiver.recv_timeout(FIRST_MESSAGE_TIMEOUT) {
        Ok(Incoming::State(state)) => *state,
        Err(RecvTimeoutError::Timeout) => PopupState {
            refreshing: true,
            view: PopupView::from_settings(&Settings::default()),
            ..Default::default()
        },
        Ok(Incoming::Disconnected) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
    };
    run(Mode::Connected, first, anchor, Some((receiver, repaint)))
}

pub fn run_demo(states: bool, light: bool) -> eframe::Result {
    run(Mode::Demo, demo_state(states, light), None, None)
}

fn parse_anchor(value: &str) -> Option<PixelPoint> {
    let mut parts = value.split(',').map(|part| part.trim().parse::<i32>());
    let point = PixelPoint { x: parts.next()?.ok()?, y: parts.next()?.ok()? };
    parts.next().is_none().then_some(point)
}

fn visible_count(state: &PopupState) -> usize {
    if state.view.show_more { state.devices.len() } else { state.devices.len().min(2) }
}

fn estimated_height(state: &PopupState) -> f32 {
    let rows = visible_count(state) as f32;
    let list = if rows == 0.0 { 36.0 } else { (rows * (ROW_MINIMUM_HEIGHT + ROW_GAP)).min(MAXIMUM_LIST_HEIGHT) };
    let footer = if state.status.is_some() { 26.0 } else { 0.0 };
    PAD_Y * 2.0 + HEADER_HEIGHT + HEADER_GAP + list + footer
}

fn run(mode: Mode, state: PopupState, anchor: Option<PixelPoint>, connection: Option<Connection>) -> eframe::Result {
    let height = estimated_height(&state) + if mode == Mode::Demo { DEMO_SLACK } else { 0.0 };
    let viewport = egui::ViewportBuilder::default()
        .with_title("Bluetooth Battery")
        .with_icon(egui::IconData::default())
        .with_inner_size([POPUP_WIDTH, height])
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_taskbar(false)
        .with_always_on_top();
    let viewport = if mode == Mode::Connected {
        viewport.with_position([-32000.0, -32000.0])
    } else {
        viewport.with_active(false)
    };
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "bluetooth-battery-monitor",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, mode, state, anchor, connection)))),
    )
}

fn window_id(frame: &eframe::Frame) -> Option<isize> {
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

fn is_routine_status(status: &str) -> bool {
    status.is_empty()
        || ["Connected", "Not charging", "Discharging"].iter().any(|routine| status.eq_ignore_ascii_case(routine))
}

fn compact(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn detail_text(device: &Device, view: &PopupView) -> Option<String> {
    let status = compact(&device.status);
    if !view.show_details {
        return device.cached.then(|| if is_routine_status(&status) { "Cached reading".to_owned() } else { status });
    }
    let mut details = Vec::new();
    let source = compact(&device.source);
    if !source.is_empty() {
        details.push(source);
    }
    if let Some(estimate) = device.estimate.as_deref().map(compact).filter(|estimate| !estimate.is_empty()) {
        details.push(estimate);
    }
    if !is_routine_status(&status) {
        details.push(status);
    } else if device.cached {
        details.push("Cached reading".to_owned());
    }
    (!details.is_empty()).then(|| details.join(" \u{00b7} "))
}

fn display_name(device: &Device) -> &str {
    if device.name.trim().is_empty() { "Wireless device" } else { device.name.trim() }
}

fn description(device: &Device) -> Vec<(&'static str, String)> {
    let mut lines = vec![(
        "Battery",
        device.percent.map_or_else(|| "unavailable".to_owned(), |percent| format!("{percent}%")),
    )];
    if !device.source.trim().is_empty() {
        lines.push(("Source", device.source.trim().to_owned()));
    }
    if !device.status.trim().is_empty() {
        lines.push(("Status", device.status.trim().to_owned()));
    }
    if device.cached && is_routine_status(&compact(&device.status)) {
        lines.push(("Reading", "Cached".to_owned()));
    }
    if let Some(estimate) = device.estimate.as_deref().filter(|estimate| !estimate.trim().is_empty()) {
        lines.push(("Rate", estimate.trim().to_owned()));
    }
    lines
}

struct RowLayout {
    name: Arc<egui::Galley>,
    detail: Option<Arc<egui::Galley>>,
    height: f32,
}

struct App {
    mode: Mode,
    state: PopupState,
    theme: Theme,
    incoming: Option<Receiver<Incoming>>,
    anchor: Option<PixelPoint>,
    desired_height: Option<f32>,
    applied_height: f32,
    placed_scale: f32,
    frames: u8,
    was_focused: bool,
    scroll_offset: f32,
    scroll_request: Option<f32>,
    list_height: f32,
    list_viewport: f32,
}

impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        mode: Mode,
        state: PopupState,
        anchor: Option<PixelPoint>,
        connection: Option<Connection>,
    ) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.set_fonts(theme::load_fonts());
        let incoming = connection.map(|(receiver, repaint)| {
            let _ = repaint.set(ctx.clone());
            receiver
        });
        let theme = Theme::resolve(&state.view.theme);
        let app = Self {
            mode,
            state,
            theme,
            incoming,
            anchor,
            desired_height: None,
            applied_height: 0.0,
            placed_scale: 0.0,
            frames: 0,
            was_focused: false,
            scroll_offset: 0.0,
            scroll_request: None,
            list_height: 0.0,
            list_viewport: 0.0,
        };
        app.apply_style(ctx);
        app
    }

    fn apply_style(&self, ctx: &egui::Context) {
        let theme = self.theme.clone();
        ctx.set_theme(if theme.dark { egui::Theme::Dark } else { egui::Theme::Light });
        ctx.all_styles_mut(move |style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
            style.visuals.window_fill = theme.popup_surface;
            style.visuals.window_stroke = Stroke::new(1.0, theme.popup_border);
            style.visuals.override_text_color = Some(theme.popup_foreground);
        });
    }

    fn send(&self, message: &FromPopup) {
        let _ = ipc::write_message(&mut std::io::stdout(), message);
    }

    fn place(&self, ctx: &egui::Context, height: f32) {
        let scale = ctx.pixels_per_point();
        let work = win::work_area(self.anchor);
        let pixels = |value: f32| (value * scale).round() as i32;
        let (width_px, height_px, margin) = (pixels(POPUP_WIDTH), pixels(height), pixels(SCREEN_MARGIN));
        let (x, y) = match self.anchor {
            Some(cursor) => {
                let above = cursor.y - height_px - pixels(ANCHOR_OFFSET_Y);
                let y = if above < work.top + margin { cursor.y + pixels(ANCHOR_OFFSET_Y) } else { above };
                (cursor.x - width_px + pixels(ANCHOR_OFFSET_X), y)
            }
            None => (work.right - width_px - margin, work.bottom - height_px - margin),
        };
        let x = x.min(work.right - width_px - margin).max(work.left + margin);
        let y = y.min(work.bottom - height_px - margin).max(work.top + margin);
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(Pos2::new(x as f32 / scale, y as f32 / scale)));
    }

    fn measure_rows(&self, painter: &egui::Painter, width: f32) -> Vec<RowLayout> {
        let info_width = width - INFO_LEFT - INFO_RIGHT_MARGIN;
        self.state
            .devices
            .iter()
            .take(visible_count(&self.state))
            .map(|device| {
                let name = theme::wrapped(
                    painter,
                    display_name(device),
                    theme::semibold(14.5),
                    self.theme.popup_foreground,
                    info_width,
                );
                let detail = detail_text(device, &self.state.view).map(|text| {
                    theme::wrapped(painter, &text, theme::regular(13.0), self.theme.popup_muted, info_width)
                });
                let block = name.size().y + detail.as_ref().map_or(0.0, |detail| 3.0 + detail.size().y);
                RowLayout { name, detail, height: ROW_MINIMUM_HEIGHT.max(block.max(40.0) + 10.0) }
            })
            .collect()
    }

    fn handle_scroll_keys(&mut self, ctx: &egui::Context) {
        let maximum = (self.list_height - self.list_viewport).max(0.0);
        let page = self.list_viewport.max(KEY_SCROLL_STEP);
        let target = ctx.input(|input| {
            let pressed = |key: egui::Key| input.key_pressed(key);
            if pressed(egui::Key::ArrowDown) {
                Some(self.scroll_offset + KEY_SCROLL_STEP)
            } else if pressed(egui::Key::ArrowUp) {
                Some(self.scroll_offset - KEY_SCROLL_STEP)
            } else if pressed(egui::Key::PageDown) {
                Some(self.scroll_offset + page)
            } else if pressed(egui::Key::PageUp) {
                Some(self.scroll_offset - page)
            } else if pressed(egui::Key::Home) {
                Some(0.0)
            } else if pressed(egui::Key::End) {
                Some(maximum)
            } else {
                None
            }
        });
        if let Some(target) = target {
            self.scroll_request = Some(target.clamp(0.0, maximum));
            ctx.request_repaint();
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let mut disconnected = false;
        let mut restyle = false;
        if let Some(receiver) = &self.incoming {
            while let Ok(incoming) = receiver.try_recv() {
                match incoming {
                    Incoming::State(state) => {
                        restyle |= state.view.theme != self.state.view.theme;
                        self.state = *state;
                    }
                    Incoming::Disconnected => disconnected = true,
                }
            }
        }
        if restyle {
            self.theme = Theme::resolve(&self.state.view.theme);
            self.apply_style(ctx);
        }
        if self.mode != Mode::Connected {
            return;
        }
        self.handle_scroll_keys(ctx);

        let Some(height) = self.desired_height else {
            ctx.request_repaint();
            return;
        };
        let resized = (height - self.applied_height).abs() > 0.5;
        if resized {
            self.applied_height = height;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(POPUP_WIDTH, height)));
        }
        let scale = ctx.pixels_per_point();
        if resized || scale != self.placed_scale {
            self.placed_scale = scale;
            self.place(ctx, height);
        }

        let focused = ctx.input(|input| input.viewport().focused.unwrap_or(false));
        let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        if focused {
            self.was_focused = true;
        } else if !self.was_focused && self.frames <= FOREGROUND_ATTEMPTS {
            if self.frames > 0
                && let Some(window) = window_id(frame)
            {
                win::set_foreground(window);
            }
            self.frames += 1;
            ctx.request_repaint();
        }
        if disconnected || escape || (self.was_focused && !focused) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let theme = self.theme.clone();
        let full = ui.max_rect();
        let painter = ui.painter().clone();
        let content_width = POPUP_WIDTH - PAD_X * 2.0;

        let mut rows = self.measure_rows(&painter, content_width);
        let total = |rows: &[RowLayout]| rows.iter().map(|row| row.height + ROW_GAP).sum::<f32>();
        let scrolling = total(&rows) > MAXIMUM_LIST_HEIGHT;
        let row_width = if scrolling { content_width - SCROLLBAR_ALLOWANCE } else { content_width };
        if scrolling {
            rows = self.measure_rows(&painter, row_width);
        }
        let list_height = total(&rows);
        let list_viewport = list_height.min(MAXIMUM_LIST_HEIGHT);

        let empty = rows.is_empty().then(|| {
            let text = if !self.state.has_snapshot || self.state.refreshing {
                "Checking devices..."
            } else {
                "No supported wireless battery devices found."
            };
            theme::wrapped(&painter, text, theme::regular(14.0), theme.popup_muted, content_width)
        });
        let footer = self.state.status.as_deref().filter(|status| !status.is_empty()).map(|status| {
            theme::wrapped(&painter, status, theme::regular(13.0), theme.popup_footer, content_width)
        });
        let body_height = empty.as_ref().map_or(list_viewport, |galley| galley.size().y + 16.0);
        let footer_height = footer.as_ref().map_or(0.0, |galley| galley.size().y + 8.0);
        let desired = (PAD_Y * 2.0 + HEADER_HEIGHT + HEADER_GAP + body_height + footer_height).ceil();
        if self.desired_height != Some(desired) {
            self.desired_height = Some(desired);
            ui.ctx().request_repaint();
        }
        self.list_height = list_height;
        self.list_viewport = list_viewport;

        let frame_rect = Rect::from_min_size(full.min, Vec2::new(POPUP_WIDTH, desired));
        painter.rect(
            frame_rect.shrink(0.5),
            CornerRadius::same(9),
            theme.popup_background,
            Stroke::new(1.0, theme.popup_border),
            StrokeKind::Middle,
        );
        let origin = Pos2::new(frame_rect.left() + PAD_X, frame_rect.top() + PAD_Y);
        let header = Rect::from_min_size(origin, Vec2::new(content_width, HEADER_HEIGHT));
        self.draw_header(ui, &painter, header);

        let body_top = header.bottom() + HEADER_GAP;
        if let Some(galley) = empty {
            painter.galley(Pos2::new(origin.x, body_top + 8.0), galley, theme.popup_muted);
        } else {
            let list_rect = Rect::from_min_size(Pos2::new(origin.x, body_top), Vec2::new(content_width, list_viewport));
            let devices: Vec<Device> = self.state.devices.iter().take(rows.len()).cloned().collect();
            let view = self.state.view.clone();
            let request = self.scroll_request.take();
            let offset = ui
                .scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
                    if let Some(request) = request {
                        area = area.vertical_scroll_offset(request);
                    }
                    area.show(ui, |ui| {
                        for (index, (device, row)) in devices.iter().zip(&rows).enumerate() {
                            let (card, response) =
                                ui.allocate_exact_size(Vec2::new(row_width, row.height), Sense::hover());
                            if ui.is_rect_visible(card) {
                                draw_row(ui.painter(), card, device, row, &theme, &view);
                            }
                            let lines = description(device);
                            let name = display_name(device).to_owned();
                            let muted = theme.popup_muted;
                            response.on_hover_ui(move |ui| {
                                ui.set_max_width(300.0);
                                ui.label(egui::RichText::new(name).font(theme::semibold(14.0)));
                                ui.add_space(4.0);
                                egui::Grid::new(("details", index)).num_columns(2).spacing([14.0, 4.0]).show(
                                    ui,
                                    |ui| {
                                        for (label, value) in lines {
                                            ui.label(egui::RichText::new(label).size(12.0).color(muted));
                                            ui.label(egui::RichText::new(value).size(12.0));
                                            ui.end_row();
                                        }
                                    },
                                );
                            });
                            ui.add_space(ROW_GAP);
                        }
                    })
                    .state
                    .clamped_offset()
                    .y
                })
                .inner;
            self.scroll_offset = offset;
        }

        if let Some(galley) = footer {
            painter.galley(Pos2::new(origin.x, body_top + body_height + 8.0), galley, theme.popup_footer);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
}

impl App {
    fn draw_header(&mut self, ui: &egui::Ui, painter: &egui::Painter, header: Rect) {
        let theme = &self.theme;
        let center_y = header.center().y;
        let logo = Rect::from_min_size(Pos2::new(header.left(), center_y - 17.0), Vec2::splat(34.0));
        painter.rect_filled(logo, CornerRadius::same(8), theme.popup_logo);
        let logo_text = if theme.high_contrast { theme.navigation_selected_text } else { egui::Color32::WHITE };
        theme::paint_icon(painter, logo.center(), theme::GLYPH_BLUETOOTH, 21.0, logo_text);

        let settings = Rect::from_min_size(Pos2::new(header.right() - 34.0, center_y - 17.0), Vec2::splat(34.0));
        let refresh = settings.translate(Vec2::new(-38.0, 0.0));
        let connected = self.mode == Mode::Connected;
        let refresh_label = if self.state.refreshing { "Checking batteries" } else { "Check batteries" };
        let refresh_clicked = header_button(
            ui,
            painter,
            theme,
            refresh,
            "refresh",
            theme::GLYPH_REFRESH,
            refresh_label,
            connected && !self.state.refreshing,
        );
        let settings_clicked =
            header_button(ui, painter, theme, settings, "settings", theme::GLYPH_SETTINGS, "Open settings", connected);

        let title_left = logo.right() + 11.0;
        let title =
            painter.layout_no_wrap("Bluetooth Battery".to_owned(), theme::semibold(16.0), theme.popup_foreground);
        let title_size = title.size();
        painter.galley(Pos2::new(title_left, center_y - title_size.y / 2.0), title, theme.popup_foreground);

        let count = self.state.devices.len();
        let mut metadata = format!("{count} {}", if count == 1 { "device" } else { "devices" });
        if self.state.view.show_updated && !self.state.updated.is_empty() {
            metadata.push_str("  |  Updated ");
            metadata.push_str(&self.state.updated);
        }
        let metadata_left = title_left + title_size.x + 10.0;
        let galley = theme::truncated(
            painter,
            &metadata,
            theme::regular(12.5),
            theme.popup_muted,
            (refresh.left() - 8.0 - metadata_left).max(0.0),
        );
        let metadata_y = center_y - galley.size().y / 2.0 + 1.0;
        painter.galley(Pos2::new(metadata_left, metadata_y), galley, theme.popup_muted);

        if refresh_clicked {
            self.state.refreshing = true;
            self.send(&FromPopup::Refresh);
        }
        if settings_clicked {
            self.send(&FromPopup::Settings);
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn header_button(
    ui: &egui::Ui,
    painter: &egui::Painter,
    theme: &Theme,
    rect: Rect,
    id: &str,
    glyph: &str,
    tooltip: &str,
    enabled: bool,
) -> bool {
    let response = ui
        .interact(rect, ui.scope_id().with(id), if enabled { Sense::click() } else { Sense::hover() })
        .on_hover_text(tooltip);
    let fill = if enabled && response.hovered() { theme.popup_action_hover } else { theme.popup_action };
    painter.rect(
        rect.shrink(0.5),
        CornerRadius::same(7),
        fill,
        Stroke::new(1.0, theme.popup_action_border),
        StrokeKind::Middle,
    );
    let color = if enabled { theme.popup_foreground } else { theme.popup_foreground.gamma_multiply(0.45) };
    theme::paint_icon(painter, rect.center(), glyph, 20.0, color);
    enabled && response.clicked()
}

fn draw_row(painter: &egui::Painter, card: Rect, device: &Device, row: &RowLayout, theme: &Theme, view: &PopupView) {
    painter.rect_filled(card, CornerRadius::same(6), theme.popup_surface);
    let center_y = card.center().y;
    let divider = (card.top() + 10.0)..=(card.bottom() - 10.0);
    let divider_stroke = Stroke::new(1.0, theme.popup_divider);

    theme::paint_icon(
        painter,
        Pos2::new(card.left() + 20.0, center_y),
        theme::device_glyph(&device.name),
        25.0,
        theme.popup_foreground,
    );
    painter.vline(card.left() + 40.5, divider.clone(), divider_stroke);

    let color = theme.battery_color(device.percent, view);
    let percent_label = device.percent.map_or_else(|| "--".to_owned(), |percent| format!("{percent}%"));
    let percent_text = painter.layout_no_wrap(percent_label, theme::semibold(22.0), theme.popup_foreground);
    let group_width = 40.0 + 8.0 + percent_text.size().x;
    let group_left = card.left() + 41.0 + (120.0 - group_width) / 2.0;
    draw_gauge(painter, Pos2::new(group_left + 20.0, center_y), device, theme, color);
    let percent_height = percent_text.size().y;
    painter.galley(
        Pos2::new(group_left + 48.0, center_y - percent_height / 2.0),
        percent_text,
        theme.popup_foreground,
    );
    painter.vline(card.left() + 161.5, divider, divider_stroke);

    let info_left = card.left() + INFO_LEFT;
    let name_height = row.name.size().y;
    let block = name_height + row.detail.as_ref().map_or(0.0, |detail| 3.0 + detail.size().y);
    let top = center_y - block / 2.0;
    painter.galley(Pos2::new(info_left, top), Arc::clone(&row.name), theme.popup_foreground);
    if let Some(detail) = &row.detail {
        painter.galley(Pos2::new(info_left, top + name_height + 3.0), Arc::clone(detail), theme.popup_muted);
    }
}

fn draw_gauge(painter: &egui::Painter, center: Pos2, device: &Device, theme: &Theme, color: egui::Color32) {
    const RADIUS: f32 = 16.0;
    const WIDTH: f32 = 4.0;
    painter.circle_stroke(center, RADIUS, Stroke::new(WIDTH, theme.popup_track));
    match device.percent {
        Some(percent) if percent >= 100 => {
            painter.circle_stroke(center, RADIUS, Stroke::new(WIDTH, color));
        }
        Some(percent) if percent > 0 => {
            let sweep = f32::from(percent) / 100.0 * std::f32::consts::TAU;
            let steps = ((sweep / std::f32::consts::TAU) * 96.0).ceil().max(2.0) as usize;
            let point =
                |angle: f32| Pos2::new(center.x + RADIUS * angle.sin(), center.y - RADIUS * angle.cos());
            let points: Vec<Pos2> = (0..=steps).map(|step| point(sweep * step as f32 / steps as f32)).collect();
            painter.circle_filled(point(0.0), WIDTH / 2.0, color);
            painter.circle_filled(point(sweep), WIDTH / 2.0, color);
            painter.add(Shape::line(points, Stroke::new(WIDTH, color)));
        }
        _ => {}
    }
    let charging = device.status.get(..8).is_some_and(|prefix| prefix.eq_ignore_ascii_case("Charging"));
    let glyph = match device.percent {
        None => theme::GLYPH_BATTERY_UNKNOWN,
        Some(_) if charging => theme::GLYPH_BATTERY_CHARGING,
        Some(_) => theme::GLYPH_BATTERY,
    };
    theme::paint_icon(painter, center, glyph, 18.0, color);
}

fn demo_state(states: bool, light: bool) -> PopupState {
    let device = |name: &str, source: &str, percent: Option<u8>, status: &str| Device {
        id: format!("demo:{name}"),
        name: name.to_owned(),
        source: source.to_owned(),
        percent,
        status: status.to_owned(),
        priority: 30,
        ..Default::default()
    };
    let mut devices = if states {
        vec![
            device("Wireless keyboard", "Logitech LIGHTSPEED", Some(1), "Discharging"),
            device("HyperX Cloud III S Wireless Gaming Headset With A Long Name", "HyperX 2.4 GHz", Some(8), "Not charging"),
            device("PRO X3 SUPERSTRIKE", "Logitech LIGHTSPEED", Some(21), "Charging slowly"),
            device("PRO X SUPERLIGHT 2c", "Logitech LIGHTSPEED", Some(100), "Charging"),
            device("Xbox Wireless Controller", "Bluetooth LE", None, "Unavailable"),
        ]
    } else {
        vec![
            device("HyperX Cloud III S Wireless", "HyperX 2.4 GHz", Some(47), "Not charging"),
            device("PRO X SUPERLIGHT 2c", "Logitech LIGHTSPEED", Some(55), "Discharging"),
        ]
    };
    if states {
        devices[0].estimate = Some("Avg. \u{2212}1.25%/h \u{00b7} ~48m remaining".to_owned());
        devices[1].cached = true;
        devices[1].status = "Last reported 11/8/2026 3:09 AM \u{00b7} sleeping or unavailable".to_owned();
        devices[2].estimate = Some("Est. +18.5%/h \u{00b7} ~4h 16m to full".to_owned());
    }
    let mut view = PopupView::from_settings(&Settings::default());
    view.theme = if light { THEME_LIGHT } else { THEME_DARK }.to_owned();
    PopupState {
        devices,
        updated: "11:57 AM".to_owned(),
        status: states.then(|| "Some device types could not be checked.".to_owned()),
        refreshing: false,
        has_snapshot: true,
        view,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(status: &str, cached: bool, estimate: Option<&str>) -> Device {
        Device {
            name: "Mouse".to_owned(),
            source: "Logitech LIGHTSPEED".to_owned(),
            percent: Some(50),
            status: status.to_owned(),
            cached,
            estimate: estimate.map(str::to_owned),
            ..Default::default()
        }
    }

    fn view(show_details: bool) -> PopupView {
        PopupView { show_details, ..PopupView::from_settings(&Settings::default()) }
    }

    #[test]
    fn details_show_source_rate_and_only_unusual_statuses() {
        let details = |status: &str, cached: bool, estimate: Option<&str>| {
            detail_text(&device(status, cached, estimate), &view(true))
        };
        assert_eq!(details("Discharging", false, None).as_deref(), Some("Logitech LIGHTSPEED"));
        assert_eq!(
            details("Discharging", false, Some("Avg. \u{2212}2%/h")).as_deref(),
            Some("Logitech LIGHTSPEED \u{00b7} Avg. \u{2212}2%/h")
        );
        assert_eq!(details("Charging", false, None).as_deref(), Some("Logitech LIGHTSPEED \u{00b7} Charging"));
        assert_eq!(details("Connected", true, None).as_deref(), Some("Logitech LIGHTSPEED \u{00b7} Cached reading"));
    }

    #[test]
    fn hiding_details_keeps_only_cached_notices() {
        let details = |status: &str, cached: bool| detail_text(&device(status, cached, Some("Avg.")), &view(false));
        assert_eq!(details("Charging", false), None);
        assert_eq!(details("Not charging", true).as_deref(), Some("Cached reading"));
        assert_eq!(details("Last reported today", true).as_deref(), Some("Last reported today"));
    }

    #[test]
    fn tooltips_list_every_known_fact() {
        let lines = description(&device("Connected", true, Some("Avg. \u{2212}2%/h")));
        let labels: Vec<&str> = lines.iter().map(|(label, _)| *label).collect();
        assert_eq!(labels, ["Battery", "Source", "Status", "Reading", "Rate"]);
        let unknown = Device { percent: None, ..Default::default() };
        assert_eq!(description(&unknown), vec![("Battery", "unavailable".to_owned())]);
    }

    #[test]
    fn anchors_and_device_limits_parse_as_expected() {
        assert_eq!(parse_anchor("-1920, 1040"), Some(PixelPoint { x: -1920, y: 1040 }));
        assert_eq!(parse_anchor("1,2,3"), None);
        assert_eq!(parse_anchor("a,2"), None);
        let mut state = demo_state(true, false);
        assert_eq!(visible_count(&state), 5);
        state.view.show_more = false;
        assert_eq!(visible_count(&state), 2);
    }
}
