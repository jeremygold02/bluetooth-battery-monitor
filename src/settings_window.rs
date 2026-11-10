use crate::icon::{ICON_SIZE, IconContent, IconFonts, IconStyle, color_for_percent, render};
use crate::ipc::{self, FromSettings, SettingsInit, ToSettings, UpdateStatus};
use crate::monitor::Device;
use crate::settings::{
    self, ICON_METRIC_AVERAGE, ICON_METRIC_DEVICE, ICON_METRIC_LOWEST, Settings, THEME_DARK, THEME_LIGHT, THEME_SYSTEM,
};
use crate::theme::{self, Theme};
use crate::win;
use eframe::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, Vec2, ViewportCommand,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::io::BufReader;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const INIT_TIMEOUT: Duration = Duration::from_secs(5);
const WINDOW_SIZE: [f32; 2] = [960.0, 640.0];
const MINIMUM_SIZE: [f32; 2] = [640.0, 420.0];
const SIDEBAR_WIDTH: f32 = 205.0;
const FOOTER_HEIGHT: f32 = 61.0;
const PAGE_NAMES: [&str; 4] = ["General", "Battery & alerts", "Tray icon", "Appearance"];
const PAGE_GLYPHS: [&str; 4] =
    [theme::GLYPH_SETTINGS, theme::GLYPH_BATTERY_PAGE, theme::GLYPH_TRAY_PAGE, theme::GLYPH_APPEARANCE_PAGE];
const BACKGROUND_SWATCHES: [[u8; 3]; 4] = [[0, 0, 0], [73, 81, 88], [0, 88, 83], [0, 194, 186]];
const TEXT_SWATCHES: [[u8; 3]; 4] = [[255, 255, 255], [211, 211, 211], [128, 128, 128], [0, 0, 0]];
const THEMES: [&str; 3] = [THEME_SYSTEM, THEME_DARK, THEME_LIGHT];

enum Incoming {
    Message(ToSettings),
    Disconnected,
}

type Connection = (Receiver<Incoming>, Arc<OnceLock<egui::Context>>);

pub fn run_connected() -> eframe::Result {
    let (sender, receiver) = mpsc::channel::<Incoming>();
    let repaint = Arc::new(OnceLock::<egui::Context>::new());
    let reader_repaint = Arc::clone(&repaint);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(std::io::stdin());
        loop {
            let incoming = match ipc::read_message::<ToSettings>(&mut reader) {
                Ok(Some(message)) => Incoming::Message(message),
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
    let Ok(Incoming::Message(ToSettings::Init(init))) = receiver.recv_timeout(INIT_TIMEOUT) else {
        return Ok(());
    };
    run(*init, 2, Some((receiver, repaint)), true)
}

pub fn run_demo(page: usize, light: bool) -> eframe::Result {
    let device = |name: &str, source: &str, percent: Option<u8>| Device {
        id: format!("demo:{name}"),
        name: name.to_owned(),
        source: source.to_owned(),
        percent,
        ..Default::default()
    };
    let mut settings = Settings { theme: if light { THEME_LIGHT } else { THEME_DARK }.to_owned(), ..Default::default() };
    settings.icon_metric = ICON_METRIC_DEVICE.to_owned();
    settings.icon_device_id = "demo:HyperX Cloud III S Wireless".to_owned();
    let init = SettingsInit {
        settings,
        start_with_windows: false,
        devices: vec![
            device("HyperX Cloud III S Wireless", "HyperX 2.4 GHz", Some(53)),
            device("PRO X SUPERLIGHT 2c", "Logitech LIGHTSPEED", Some(79)),
        ],
        version: crate::updater::build_version().to_owned(),
        update: UpdateStatus {
            text: "Updates have not been checked yet.".to_owned(),
            busy: false,
            button: "Check for updates".to_owned(),
        },
        save_blocked: None,
    };
    run(init, page.min(3), None, false)
}

fn run(init: SettingsInit, page: usize, connection: Option<Connection>, active: bool) -> eframe::Result {
    let viewport = egui::ViewportBuilder::default()
        .with_title("Bluetooth Battery Monitor Settings")
        .with_icon(egui::IconData::default())
        .with_inner_size(WINDOW_SIZE)
        .with_min_inner_size(MINIMUM_SIZE)
        .with_active(active);
    let options = eframe::NativeOptions { viewport, centered: true, ..Default::default() };
    eframe::run_native(
        "bluetooth-battery-monitor-settings",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, init, page, connection)))),
    )
}

struct NumberField {
    text: String,
    value: i32,
    minimum: i32,
    maximum: i32,
}

impl NumberField {
    fn new(value: i32, minimum: i32, maximum: i32) -> Self {
        let value = value.clamp(minimum, maximum);
        Self { text: value.to_string(), value, minimum, maximum }
    }

    fn set(&mut self, value: i32) {
        self.value = value.clamp(self.minimum, self.maximum);
        self.text = self.value.to_string();
    }

    fn parsed(&self) -> Option<i32> {
        self.text.trim().parse::<i32>().ok().filter(|value| (self.minimum..=self.maximum).contains(value))
    }

    fn pending(&self) -> bool {
        self.parsed() != Some(self.value)
    }

    fn accept_text(&mut self) -> bool {
        match self.parsed() {
            Some(value) if value != self.value => {
                self.value = value;
                true
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Metric {
    Lowest,
    Average,
    Device,
}

struct Choice {
    id: String,
    name: String,
    label: String,
}

#[derive(Clone, Copy, PartialEq)]
enum ColorTarget {
    Background,
    Text,
}

struct Form {
    base: Settings,
    refresh: NumberField,
    critical: NumberField,
    low: NumberField,
    preview: NumberField,
    metric: Metric,
    choices: Vec<Choice>,
    selected: Option<usize>,
    color_bars: bool,
    show_details: bool,
    show_updated: bool,
    show_more: bool,
    start_with_windows: bool,
    notifications: bool,
    show_tray_box: bool,
    use_custom_box: bool,
    box_color: [u8; 3],
    text_color: [u8; 3],
    theme: String,
}

fn build_choices(devices: &[Device], selected_id: &str, selected_name: &str) -> (Vec<Choice>, Option<usize>) {
    let mut sorted: Vec<&Device> = devices.iter().filter(|device| !device.id.trim().is_empty()).collect();
    sorted.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.to_lowercase().cmp(&right.id.to_lowercase()))
    });
    let mut choices: Vec<Choice> = Vec::new();
    for device in sorted {
        if choices.iter().any(|choice| choice.id.to_lowercase() == device.id.to_lowercase()) {
            continue;
        }
        let name = if device.name.trim().is_empty() { "Wireless device" } else { device.name.as_str() };
        let mut label = name.to_owned();
        if !device.source.trim().is_empty() {
            label.push_str(&format!(" ({})", device.source));
        }
        let same_name = choices.iter().filter(|choice| choice.name.to_lowercase() == name.to_lowercase()).count();
        if same_name > 0 {
            label.push_str(&format!(" #{}", same_name + 1));
        }
        if device.cached || device.percent.is_none() {
            label.push_str(" (unavailable)");
        }
        choices.push(Choice { id: device.id.clone(), name: name.to_owned(), label });
    }
    let position = |choices: &[Choice]| {
        choices.iter().position(|choice| choice.id.to_lowercase() == selected_id.to_lowercase())
    };
    if !selected_id.is_empty() && position(&choices).is_none() {
        let name = if selected_name.trim().is_empty() { "Selected device" } else { selected_name };
        choices.push(Choice { id: selected_id.to_owned(), name: name.to_owned(), label: format!("{name} (unavailable)") });
    }
    let selected = position(&choices).or((!choices.is_empty()).then_some(0));
    (choices, selected)
}

fn linear_channel(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.03928 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
}

fn luminance(color: [u8; 3]) -> f64 {
    0.2126 * linear_channel(color[0]) + 0.7152 * linear_channel(color[1]) + 0.0722 * linear_channel(color[2])
}

fn contrast_ratio(first: [u8; 3], second: [u8; 3]) -> f64 {
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn contrast_guidance(show_box: bool, text: [u8; 3], background: [u8; 3]) -> (String, bool) {
    if !show_box {
        return ("Background hidden: use text that contrasts with the taskbar.".to_owned(), false);
    }
    let ratio = contrast_ratio(text, background);
    let recommended =
        if contrast_ratio([0; 3], background) >= contrast_ratio([255; 3], background) { [0; 3] } else { [255; 3] };
    let low = ratio < 4.5;
    (
        format!(
            "{} {ratio:.2}:1. Recommended text: {}.",
            if low { "Low contrast" } else { "Contrast" },
            settings::format_color(recommended)
        ),
        low,
    )
}

impl Form {
    fn new(base: Settings, start_with_windows: bool, devices: &[Device]) -> Self {
        let metric = if base.icon_metric.eq_ignore_ascii_case(ICON_METRIC_DEVICE) {
            Metric::Device
        } else if base.icon_metric.eq_ignore_ascii_case(ICON_METRIC_AVERAGE) {
            Metric::Average
        } else {
            Metric::Lowest
        };
        let (choices, selected) = build_choices(devices, &base.icon_device_id, &base.icon_device_name);
        let mut form = Self {
            refresh: NumberField::new(base.refresh_seconds, 30, 3600),
            critical: NumberField::new(base.critical_threshold, 1, 99),
            low: NumberField::new(base.low_threshold, 1, 99),
            preview: NumberField::new(50, 0, 100),
            metric,
            choices,
            selected,
            color_bars: base.color_bars,
            show_details: base.show_popup_reset_times,
            show_updated: base.show_popup_last_updated,
            show_more: base.show_reset_availability,
            start_with_windows,
            notifications: base.threshold_notifications,
            show_tray_box: base.show_tray_box,
            use_custom_box: base.use_custom_tray_box_color,
            box_color: base.tray_box_rgb(),
            text_color: base.tray_text_rgb(),
            theme: base.theme.clone(),
            base,
        };
        form.ensure_valid();
        form
    }

    fn ensure_valid(&mut self) {
        if self.critical.value > self.low.value {
            self.low.set(self.critical.value);
        }
    }

    fn update_devices(&mut self, devices: &[Device]) {
        let (id, name) = match self.selected.and_then(|index| self.choices.get(index)) {
            Some(choice) => (choice.id.clone(), choice.name.clone()),
            None => (self.base.icon_device_id.clone(), self.base.icon_device_name.clone()),
        };
        (self.choices, self.selected) = build_choices(devices, &id, &name);
    }

    fn candidate(&self) -> Settings {
        let mut candidate = self.base.clone();
        candidate.critical_threshold = self.critical.value;
        candidate.low_threshold = self.low.value;
        candidate.icon_metric = match self.metric {
            Metric::Device => ICON_METRIC_DEVICE,
            Metric::Average => ICON_METRIC_AVERAGE,
            Metric::Lowest => ICON_METRIC_LOWEST,
        }
        .to_owned();
        if self.metric == Metric::Device
            && let Some(choice) = self.selected.and_then(|index| self.choices.get(index))
        {
            candidate.icon_device_id = choice.id.clone();
            candidate.icon_device_name = choice.name.clone();
        }
        candidate.color_bars = self.color_bars;
        candidate.show_popup_reset_times = self.show_details;
        candidate.show_popup_last_updated = self.show_updated;
        candidate.show_reset_availability = self.show_more;
        candidate.start_with_windows = self.start_with_windows;
        candidate.threshold_notifications = self.notifications;
        candidate.show_tray_box = self.show_tray_box;
        candidate.use_custom_tray_box_color = self.use_custom_box;
        candidate.tray_box_color = settings::format_color(self.box_color);
        candidate.tray_text_color = settings::format_color(self.text_color);
        candidate.refresh_seconds = self.refresh.value;
        candidate.theme = self.theme.clone();
        candidate
    }

    fn pending_numbers(&self) -> bool {
        self.refresh.pending() || self.critical.pending() || self.low.pending()
    }

    fn restore_defaults(&mut self) {
        let defaults = Settings::default();
        self.critical.set(defaults.critical_threshold);
        self.low.set(defaults.low_threshold);
        self.refresh.set(defaults.refresh_seconds);
        self.metric = Metric::Lowest;
        self.color_bars = defaults.color_bars;
        self.show_details = defaults.show_popup_reset_times;
        self.show_updated = defaults.show_popup_last_updated;
        self.show_more = defaults.show_reset_availability;
        self.start_with_windows = defaults.start_with_windows;
        self.notifications = defaults.threshold_notifications;
        self.show_tray_box = defaults.show_tray_box;
        self.use_custom_box = defaults.use_custom_tray_box_color;
        self.box_color = defaults.tray_box_rgb();
        self.text_color = defaults.tray_text_rgb();
        self.preview.set(50);
        self.theme = defaults.theme;
    }

    fn preview_style(&self) -> IconStyle {
        IconStyle {
            show_box: self.show_tray_box,
            custom_box: self.use_custom_box.then_some(self.box_color),
            text: self.text_color,
            critical: self.critical.value,
            low: self.low.value,
        }
    }

    fn effective_background(&self) -> [u8; 3] {
        if self.use_custom_box {
            self.box_color
        } else {
            color_for_percent(self.preview.value.clamp(0, 100) as u8, self.critical.value, self.low.value)
        }
    }
}

struct App {
    form: Form,
    saved: Settings,
    theme: Theme,
    applied_theme: String,
    page: usize,
    version: String,
    update: UpdateStatus,
    save_blocked: Option<String>,
    message: Option<String>,
    saving: bool,
    reported_dirty: bool,
    incoming: Option<Receiver<Incoming>>,
    fonts: IconFonts,
    preview: Option<(IconContent, IconStyle, TextureHandle)>,
    picking: Option<(ColorTarget, Color32)>,
    icon_applied: bool,
}

fn window_id(frame: &eframe::Frame) -> Option<isize> {
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

fn color32(color: [u8; 3]) -> Color32 {
    Color32::from_rgb(color[0], color[1], color[2])
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, init: SettingsInit, page: usize, connection: Option<Connection>) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.set_fonts(theme::load_fonts());
        let incoming = connection.map(|(receiver, repaint)| {
            let _ = repaint.set(ctx.clone());
            receiver
        });
        let form = Form::new(init.settings, init.start_with_windows, &init.devices);
        let saved = form.candidate();
        let mut app = Self {
            theme: Theme::resolve(&form.theme),
            applied_theme: String::new(),
            form,
            saved,
            page,
            version: init.version,
            update: init.update,
            save_blocked: init.save_blocked,
            message: None,
            saving: false,
            reported_dirty: false,
            incoming,
            fonts: IconFonts::load(),
            preview: None,
            picking: None,
            icon_applied: false,
        };
        app.apply_style(ctx);
        app
    }

    fn apply_style(&mut self, ctx: &egui::Context) {
        self.theme = Theme::resolve(&self.form.theme);
        self.applied_theme = self.form.theme.clone();
        let theme = self.theme.clone();
        ctx.set_theme(if theme.dark { egui::Theme::Dark } else { egui::Theme::Light });
        ctx.all_styles_mut(move |style| {
            style.text_styles.insert(egui::TextStyle::Body, theme::regular(15.0));
            style.text_styles.insert(egui::TextStyle::Button, theme::regular(15.0));
            style.spacing.item_spacing = Vec2::new(6.0, 4.0);
            style.spacing.interact_size.y = 32.0;
            style.spacing.button_padding = Vec2::new(10.0, 6.0);
            let visuals = &mut style.visuals;
            visuals.override_text_color = Some(theme.foreground);
            visuals.window_fill = theme.surface;
            visuals.window_stroke = Stroke::new(1.0, theme.border);
            visuals.panel_fill = theme.base;
            visuals.extreme_bg_color = theme.control;
            visuals.selection.bg_fill = theme.accent;
            visuals.selection.stroke = Stroke::new(1.0, theme.primary_text);
            for (widget, fill) in [
                (&mut visuals.widgets.inactive, theme.control),
                (&mut visuals.widgets.hovered, theme.control_hover),
                (&mut visuals.widgets.active, theme.control_hover),
                (&mut visuals.widgets.open, theme.control_hover),
            ] {
                widget.bg_fill = fill;
                widget.weak_bg_fill = fill;
                widget.bg_stroke = Stroke::new(1.0, theme.border);
                widget.fg_stroke = Stroke::new(1.0, theme.foreground);
                widget.corner_radius = CornerRadius::same(4);
            }
        });
    }

    fn dirty(&self) -> bool {
        self.picking.is_some()
            || (self.form.metric == Metric::Device && self.form.selected.is_none())
            || !self.saved.editable_equals(&self.form.candidate())
            || self.form.pending_numbers()
    }

    fn send(&self, message: &FromSettings) {
        let _ = ipc::write_message(&mut std::io::stdout(), message);
    }

    fn save(&mut self) {
        if self.saving {
            return;
        }
        if let Some(message) = &self.save_blocked {
            self.message = Some(message.clone());
            return;
        }
        self.form.ensure_valid();
        if self.form.metric == Metric::Device && self.form.selected.is_none() {
            self.message = Some("Choose a device for the tray icon.".to_owned());
            return;
        }
        if self.incoming.is_none() {
            return;
        }
        self.message = None;
        self.saving = true;
        self.send(&FromSettings::Save(Box::new(self.form.candidate())));
    }

    fn receive(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let mut messages = Vec::new();
        if let Some(receiver) = &self.incoming {
            while let Ok(incoming) = receiver.try_recv() {
                messages.push(incoming);
            }
        }
        for incoming in messages {
            match incoming {
                Incoming::Disconnected => ctx.send_viewport_cmd(ViewportCommand::Close),
                Incoming::Message(ToSettings::Init(_)) => {}
                Incoming::Message(ToSettings::Devices(devices)) => self.form.update_devices(&devices),
                Incoming::Message(ToSettings::Update(update)) => self.update = update,
                Incoming::Message(ToSettings::SaveResult(None)) => {
                    self.saved = self.form.candidate();
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                Incoming::Message(ToSettings::SaveResult(Some(error))) => {
                    self.saving = false;
                    self.message = Some(error);
                }
                Incoming::Message(ToSettings::Activate) => {
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    if let Some(window) = window_id(frame) {
                        win::set_foreground(window);
                    }
                }
            }
        }
    }

    fn preview_texture(&mut self, ctx: &egui::Context) -> egui::TextureId {
        let content = IconContent::Percent(self.form.preview.value.clamp(0, 100) as u8);
        let style = self.form.preview_style();
        if let Some((shown, shown_style, texture)) = &self.preview
            && *shown == content
            && *shown_style == style
        {
            return texture.id();
        }
        let pixels = render(content, &style, &self.fonts);
        let image = egui::ColorImage::from_rgba_unmultiplied([ICON_SIZE, ICON_SIZE], &pixels);
        let texture = ctx.load_texture("tray-preview", image, egui::TextureOptions::NEAREST);
        let id = texture.id();
        self.preview = Some((content, style, texture));
        id
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.receive(ctx, frame);
        if !self.icon_applied
            && let Some(window) = window_id(frame)
        {
            win::apply_app_icon(window);
            self.icon_applied = true;
        }
        if self.applied_theme != self.form.theme {
            self.apply_style(ctx);
        }
        let dirty = self.dirty();
        if dirty != self.reported_dirty {
            self.reported_dirty = dirty;
            self.send(&FromSettings::Dirty(dirty));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let theme = self.theme.clone();
        let full = ui.max_rect();
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, theme.base);
        let sidebar = Rect::from_min_max(full.min, Pos2::new(full.left() + SIDEBAR_WIDTH, full.bottom()));
        painter.rect_filled(sidebar, 0.0, theme.sidebar);
        let footer = Rect::from_min_max(Pos2::new(sidebar.right(), full.bottom() - FOOTER_HEIGHT), full.max);
        painter.hline(footer.x_range(), footer.top() + 0.5, Stroke::new(1.0, theme.border));
        let content = Rect::from_min_max(Pos2::new(sidebar.right(), full.top()), Pos2::new(full.right(), footer.top()));

        for (index, name) in PAGE_NAMES.iter().enumerate() {
            let rect = Rect::from_min_size(
                Pos2::new(sidebar.left() + 12.0, sidebar.top() + 24.0 + index as f32 * 48.0),
                Vec2::new(SIDEBAR_WIDTH - 24.0, 43.0),
            );
            let response = ui.interact(rect, ui.scope_id().with(("page", index)), Sense::click());
            let selected = self.page == index;
            if selected {
                painter.rect_filled(rect, CornerRadius::same(4), theme.navigation_selected);
            } else if response.hovered() {
                painter.rect_filled(rect, CornerRadius::same(4), theme.control);
            }
            let color = if selected { theme.navigation_selected_text } else { theme.foreground };
            theme::paint_icon(&painter, Pos2::new(rect.left() + 21.0, rect.center().y), PAGE_GLYPHS[index], 17.0, color);
            painter.text(
                Pos2::new(rect.left() + 40.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                name,
                theme::regular(15.0),
                color,
            );
            if response.clicked() {
                self.page = index;
            }
        }

        let page_rect = Rect::from_min_max(
            Pos2::new(content.left() + 26.0, content.top()),
            Pos2::new(content.right() - 12.0, content.bottom()),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(page_rect), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width() - 14.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(20.0);
                match self.page {
                    0 => self.general_page(ui, &theme),
                    1 => self.alerts_page(ui, &theme),
                    2 => self.tray_page(ui, &theme),
                    _ => self.appearance_page(ui, &theme),
                }
                ui.add_space(16.0);
            });
        });

        let buttons_y = footer.top() + 13.0;
        let button_rect = |left: f32| Rect::from_min_size(Pos2::new(left, buttons_y), Vec2::new(100.0, 36.0));
        if push_button(ui, &theme, button_rect(footer.left() + 26.0), "defaults", "Defaults", false, true) {
            self.form.restore_defaults();
        }
        let save_rect = button_rect(footer.right() - 26.0 - 100.0);
        let cancel_rect = button_rect(save_rect.left() - 110.0);
        let escape = self.picking.is_none() && ui.input(|input| input.key_pressed(egui::Key::Escape));
        if push_button(ui, &theme, cancel_rect, "cancel", "Cancel", false, true) || escape {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
        let label = if self.saving { "Saving..." } else { "Save" };
        let enter = self.picking.is_none()
            && ui.memory(|memory| memory.focused().is_none())
            && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if push_button(ui, &theme, save_rect, "save", label, true, !self.saving) || enter {
            self.save();
        }
        if let Some(message) = &self.message {
            let left = footer.left() + 26.0 + 100.0 + 14.0;
            let galley = theme::truncated(
                &painter,
                &message.replace('\n', " "),
                theme::regular(13.0),
                theme.warning,
                (cancel_rect.left() - 14.0 - left).max(0.0),
            );
            let position = Pos2::new(left, footer.center().y - galley.size().y / 2.0);
            let rect = Rect::from_min_size(position, galley.size());
            painter.galley(position, galley, theme.warning);
            ui.interact(rect, ui.scope_id().with("message"), Sense::hover()).on_hover_text(message.as_str());
        }

        self.color_dialog(ui.ctx(), &theme);
    }
}

fn label(ui: &mut egui::Ui, text: &str, size: f32, color: Color32, bold: bool) {
    let font = if bold { theme::semibold(size.max(14.0)) } else { theme::regular(size.max(14.0)) };
    ui.label(egui::RichText::new(text).font(font).color(color));
}

fn heading(ui: &mut egui::Ui, theme: &Theme, title: &str, description: &str) {
    ui.label(egui::RichText::new(title).font(theme::semibold(25.0)).color(theme.foreground));
    ui.add_space(3.0);
    label(ui, description, 14.0, theme.secondary, false);
    ui.add_space(18.0);
}

fn card(ui: &mut egui::Ui, theme: &Theme, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(theme.surface)
        .stroke(Stroke::new(1.0, theme.border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(egui::Margin::symmetric(20, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            contents(ui);
        });
    ui.add_space(12.0);
}

fn push_button(ui: &egui::Ui, theme: &Theme, rect: Rect, id: &str, text: &str, primary: bool, enabled: bool) -> bool {
    let response =
        ui.interact(rect, ui.scope_id().with(("button", id)), if enabled { Sense::click() } else { Sense::hover() });
    let (fill, border, color) = if primary {
        (theme.primary, theme.primary_border, theme.primary_text)
    } else if enabled && response.hovered() {
        (theme.control_hover, theme.border, theme.foreground)
    } else {
        (theme.control, theme.border, theme.foreground)
    };
    let fade = if enabled { 1.0 } else { 0.5 };
    ui.painter().rect(
        rect.shrink(0.5),
        CornerRadius::same(4),
        fill.gamma_multiply(fade),
        Stroke::new(1.0, border),
        StrokeKind::Middle,
    );
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, theme::regular(15.0), color.gamma_multiply(fade));
    enabled && response.clicked()
}

fn inline_button(ui: &mut egui::Ui, theme: &Theme, text: &str, selected: bool, enabled: bool) -> bool {
    let width = ui.painter().layout_no_wrap(text.to_owned(), theme::regular(15.0), theme.foreground).size().x + 20.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if enabled && response.hovered() { theme.control_hover } else { theme.control };
    let stroke = if selected { Stroke::new(2.0, theme.accent) } else { Stroke::new(1.0, theme.border) };
    let fade = if enabled { 1.0 } else { 0.5 };
    ui.painter().rect(rect.shrink(1.0), CornerRadius::same(4), fill, stroke, StrokeKind::Middle);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        theme::regular(15.0),
        theme.foreground.gamma_multiply(fade),
    );
    enabled && response.clicked()
}

fn swatch(ui: &mut egui::Ui, theme: &Theme, color: [u8; 3], selected: bool, tooltip: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(32.0, 30.0), Sense::click());
    let stroke = if selected { Stroke::new(2.0, theme.accent) } else { Stroke::new(1.0, theme.border) };
    ui.painter().rect(rect.shrink(1.0), CornerRadius::same(4), color32(color), stroke, StrokeKind::Middle);
    response.on_hover_text(tooltip).clicked()
}

fn check(ui: &mut egui::Ui, theme: &Theme, value: &mut bool, text: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), theme::regular(15.0), theme.foreground);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(30.0 + galley.size().x, 28.0), Sense::click());
    if response.clicked() {
        *value = !*value;
    }
    let mark = Rect::from_center_size(Pos2::new(rect.left() + 10.0, rect.center().y), Vec2::splat(20.0));
    if *value {
        ui.painter().rect_filled(mark, CornerRadius::same(4), theme.accent);
        theme::paint_icon(ui.painter(), mark.center(), theme::GLYPH_CHECK, 12.0, theme.primary_text);
    } else {
        let fill = if response.hovered() { theme.control_hover } else { theme.control };
        ui.painter().rect(
            mark.shrink(0.5),
            CornerRadius::same(4),
            fill,
            Stroke::new(1.0, theme.secondary),
            StrokeKind::Middle,
        );
    }
    let text_position = Pos2::new(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_position, galley, theme.foreground);
    response.clicked()
}

fn radio(ui: &mut egui::Ui, theme: &Theme, selected: bool, title: &str, description: &str) -> bool {
    let title_galley = ui.painter().layout_no_wrap(title.to_owned(), theme::semibold(16.0), theme.foreground);
    let description_galley =
        ui.painter().layout_no_wrap(description.to_owned(), theme::regular(14.0), theme.secondary);
    let height = title_galley.size().y + description_galley.size().y;
    let width = 28.0 + title_galley.size().x.max(description_galley.size().x);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height + 5.0), Sense::click());
    let center = Pos2::new(rect.left() + 10.0, rect.top() + height / 2.0);
    if selected {
        ui.painter().circle_filled(center, 10.0, theme.accent);
        ui.painter().circle_filled(center, 5.0, theme.surface);
    } else {
        let fill = if response.hovered() { theme.control_hover } else { theme.control };
        ui.painter().circle(center, 9.5, fill, Stroke::new(1.0, theme.secondary));
    }
    let title_height = title_galley.size().y;
    ui.painter().galley(Pos2::new(rect.left() + 28.0, rect.top()), title_galley, theme.foreground);
    ui.painter().galley(Pos2::new(rect.left() + 28.0, rect.top() + title_height), description_galley, theme.secondary);
    response.clicked()
}

fn number(ui: &mut egui::Ui, theme: &Theme, field: &mut NumberField, id: &str, width: f32) -> bool {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::hover());
    ui.painter().rect(
        rect.shrink(0.5),
        CornerRadius::same(4),
        theme.control,
        Stroke::new(1.0, theme.border),
        StrokeKind::Middle,
    );
    let mut changed = false;
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + 10.0, rect.top() + 3.0),
        Pos2::new(rect.right() - 54.0, rect.bottom() - 5.0),
    );
    let response = ui.put(
        text_rect,
        egui::TextEdit::singleline(&mut field.text)
            .id_salt(("number", id))
            .frame(egui::Frame::NONE)
            .font(theme::regular(15.0))
            .text_color(theme.foreground)
            .align(egui::Align2::LEFT_CENTER),
    );
    if response.changed() {
        field.text.retain(|character| character.is_ascii_digit());
        changed |= field.accept_text();
    }
    if response.lost_focus() {
        field.set(field.value);
    }
    for (offset, glyph, step) in [(52.0, theme::GLYPH_CHEVRON_UP, 1), (27.0, theme::GLYPH_CHEVRON_DOWN, -1)] {
        let button = Rect::from_min_size(Pos2::new(rect.right() - offset, rect.top() + 4.0), Vec2::new(24.0, 24.0));
        let response = ui.interact(button, ui.scope_id().with(("step", id, step)), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(button, CornerRadius::same(4), theme.control_hover);
        }
        theme::paint_icon(ui.painter(), button.center(), glyph, 11.0, theme.secondary);
        if response.clicked() {
            field.set(field.value.saturating_add(step));
            changed = true;
        }
    }
    ui.advance_cursor_after_rect(rect);
    changed
}

fn fixed_label(ui: &mut egui::Ui, theme: &Theme, text: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::hover());
    ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, text, theme::regular(14.0), theme.secondary);
}

impl App {
    fn general_page(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        heading(ui, theme, "General", "Choose how often devices are checked and how the app starts.");
        card(ui, theme, |ui| {
            label(ui, "Battery checks", 15.0, theme.foreground, true);
            label(ui, "Check connected devices at this interval.", 13.0, theme.secondary, false);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                number(ui, theme, &mut self.form.refresh, "refresh", 145.0);
                label(ui, "seconds", 15.0, theme.secondary, false);
            });
        });
        card(ui, theme, |ui| {
            label(ui, "Startup", 15.0, theme.foreground, true);
            ui.add_space(8.0);
            check(ui, theme, &mut self.form.start_with_windows, "Start with Windows");
        });
        card(ui, theme, |ui| {
            label(ui, "About", 15.0, theme.foreground, true);
            ui.add_space(8.0);
            label(ui, &format!("Version {}", self.version), 13.0, theme.secondary, false);
            label(ui, &self.update.text, 13.0, theme.secondary, false);
            ui.add_space(10.0);
            let text = if self.update.button.trim().is_empty() { "Check for updates" } else { &self.update.button };
            if inline_button(ui, theme, text, false, !self.update.busy && self.incoming.is_some()) {
                self.send(&FromSettings::CheckUpdates);
            }
        });
    }

    fn alerts_page(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        heading(ui, theme, "Battery & alerts", "Notifications and battery warning thresholds.");
        card(ui, theme, |ui| {
            label(ui, "Battery notifications", 15.0, theme.foreground, true);
            label(ui, "Show an alert when a device reaches a threshold.", 13.0, theme.secondary, false);
            ui.add_space(8.0);
            check(ui, theme, &mut self.form.notifications, "Notify at thresholds");
        });
        card(ui, theme, |ui| {
            label(ui, "Battery thresholds", 15.0, theme.foreground, true);
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                label(ui, "Critical level", 14.0, theme.secondary, false);
                if number(ui, theme, &mut self.form.critical, "critical", 145.0)
                    && self.form.critical.value > self.form.low.value
                {
                    self.form.low.set(self.form.critical.value);
                }
                label(ui, "%", 14.0, theme.secondary, false);
                ui.add_space(16.0);
                label(ui, "Low level", 14.0, theme.secondary, false);
                if number(ui, theme, &mut self.form.low, "low", 145.0)
                    && self.form.low.value < self.form.critical.value
                {
                    self.form.critical.set(self.form.low.value);
                }
                label(ui, "%", 14.0, theme.secondary, false);
            });
        });
    }

    fn tray_page(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        heading(ui, theme, "Tray icon", "Customize how the battery level is shown in the system tray.");
        card(ui, theme, |ui| {
            let options = [
                (Metric::Lowest, "Lowest battery", "Show the lowest battery level across your devices."),
                (Metric::Average, "Average battery", "Show the average battery level across your devices."),
                (Metric::Device, "Specific device", "Show the battery level for a chosen device."),
            ];
            for (metric, title, description) in options {
                if radio(ui, theme, self.form.metric == metric, title, description) {
                    self.form.metric = metric;
                }
            }
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.add_enabled_ui(self.form.metric == Metric::Device, |ui| {
                    let current = self
                        .form
                        .selected
                        .and_then(|index| self.form.choices.get(index))
                        .map_or(String::new(), |choice| choice.label.clone());
                    egui::ComboBox::from_id_salt("tray-device").selected_text(current).width(330.0).show_ui(ui, |ui| {
                        for (index, choice) in self.form.choices.iter().enumerate() {
                            if ui.selectable_label(self.form.selected == Some(index), choice.label.as_str()).clicked() {
                                self.form.selected = Some(index);
                            }
                        }
                    });
                });
            });
        });

        let texture = self.preview_texture(ui.ctx());
        let background = self.form.effective_background();
        card(ui, theme, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(155.0);
                    label(ui, "LIVE PREVIEW", 12.0, theme.secondary, true);
                    ui.add_space(10.0);
                    let (frame, _) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::hover());
                    ui.painter().rect(
                        frame.shrink(0.5),
                        CornerRadius::same(5),
                        theme.preview_frame,
                        Stroke::new(1.0, theme.border),
                        StrokeKind::Middle,
                    );
                    ui.painter().image(
                        texture,
                        Rect::from_center_size(frame.center(), Vec2::splat(36.0)),
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    ui.add_space(10.0);
                    label(ui, "Battery preview", 14.0, theme.secondary, false);
                    ui.add_space(2.0);
                    number(ui, theme, &mut self.form.preview, "preview", 130.0);
                });
                ui.add_space(15.0);
                ui.vertical(|ui| {
                    check(ui, theme, &mut self.form.show_tray_box, "Show tray background");
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        fixed_label(ui, theme, "Background color", 124.0);
                        if inline_button(ui, theme, "Auto", !self.form.use_custom_box, true) {
                            self.form.use_custom_box = false;
                        }
                        let names = ["Black background", "Slate background", "Dark teal background", "Teal background"];
                        for (color, name) in BACKGROUND_SWATCHES.iter().zip(names) {
                            let selected = self.form.use_custom_box && self.form.box_color == *color;
                            if swatch(ui, theme, *color, selected, name) {
                                self.form.box_color = *color;
                                self.form.use_custom_box = true;
                            }
                        }
                        if inline_button(ui, theme, "Custom\u{2026}", false, true) {
                            self.picking = Some((ColorTarget::Background, color32(self.form.box_color)));
                        }
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        fixed_label(ui, theme, "Text color", 124.0);
                        let names = ["White text", "Light gray text", "Gray text", "Black text"];
                        for (color, name) in TEXT_SWATCHES.iter().zip(names) {
                            if swatch(ui, theme, *color, self.form.text_color == *color, name) {
                                self.form.text_color = *color;
                            }
                        }
                        if inline_button(ui, theme, "Custom\u{2026}", false, true) {
                            self.picking = Some((ColorTarget::Text, color32(self.form.text_color)));
                        }
                    });
                    ui.add_space(2.0);
                    let (text, low) = contrast_guidance(self.form.show_tray_box, self.form.text_color, background);
                    label(ui, &text, 12.0, if low { theme.warning } else { theme.secondary }, false);
                });
            });
        });

        card(ui, theme, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    label(ui, "Theme", 15.0, theme.foreground, true);
                    label(ui, "Choose the app color theme.", 14.0, theme.secondary, false);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("theme").selected_text(self.form.theme.clone()).width(206.0).show_ui(
                        ui,
                        |ui| {
                            for name in THEMES {
                                if ui.selectable_label(self.form.theme == name, name).clicked() {
                                    self.form.theme = name.to_owned();
                                }
                            }
                        },
                    );
                });
            });
        });
    }

    fn appearance_page(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        heading(ui, theme, "Appearance", "Choose which details appear in the battery popup.");
        card(ui, theme, |ui| {
            label(ui, "Battery colors", 15.0, theme.foreground, true);
            ui.add_space(8.0);
            check(ui, theme, &mut self.form.color_bars, "Color by battery level");
            ui.add_space(5.0);
            label(ui, "Use the warning thresholds to color battery indicators.", 13.0, theme.secondary, false);
        });
        card(ui, theme, |ui| {
            label(ui, "Popup details", 15.0, theme.foreground, true);
            ui.add_space(8.0);
            check(ui, theme, &mut self.form.show_more, "Show more devices");
            check(ui, theme, &mut self.form.show_details, "Show device details");
            check(ui, theme, &mut self.form.show_updated, "Show updated time");
        });
    }

    fn color_dialog(&mut self, ctx: &egui::Context, theme: &Theme) {
        let Some((target, mut color)) = self.picking else { return };
        let mut decision = None;
        egui::Window::new(if target == ColorTarget::Background { "Background color" } else { "Text color" })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::Opaque);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if inline_button(ui, theme, "OK", true, true) {
                        decision = Some(true);
                    }
                    if inline_button(ui, theme, "Cancel", false, true) {
                        decision = Some(false);
                    }
                });
            });
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            decision = Some(false);
        }
        match decision {
            Some(true) => {
                let chosen = [color.r(), color.g(), color.b()];
                match target {
                    ColorTarget::Background => {
                        self.form.box_color = chosen;
                        self.form.use_custom_box = true;
                    }
                    ColorTarget::Text => self.form.text_color = chosen,
                }
                self.picking = None;
            }
            Some(false) => self.picking = None,
            None => self.picking = Some((target, color)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str, source: &str, percent: Option<u8>, cached: bool) -> Device {
        Device {
            id: id.to_owned(),
            name: name.to_owned(),
            source: source.to_owned(),
            percent,
            cached,
            ..Default::default()
        }
    }

    #[test]
    fn device_choices_are_sorted_labelled_and_keep_a_missing_selection() {
        let devices = [
            device("b", "Mouse", "Logitech LIGHTSPEED", Some(80), false),
            device("a", "mouse", "Logitech LIGHTSPEED", None, false),
            device("B", "Mouse", "Logitech LIGHTSPEED", Some(80), false),
            device("c", "", "", Some(10), true),
            device(" ", "Ignored", "", Some(10), false),
        ];
        let (choices, selected) = build_choices(&devices, "gone", "Old headset");
        let labels: Vec<&str> = choices.iter().map(|choice| choice.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Wireless device (unavailable)",
                "mouse (Logitech LIGHTSPEED) (unavailable)",
                "Mouse (Logitech LIGHTSPEED) #2",
                "Old headset (unavailable)",
            ]
        );
        assert_eq!(selected, Some(3));
        assert_eq!(build_choices(&devices, "B", "").1, Some(2));
        assert_eq!(build_choices(&devices, "", "").1, Some(0));
        assert_eq!(build_choices(&[], "", "").1, None);
    }

    #[test]
    fn contrast_guidance_matches_the_original_wording() {
        assert_eq!(
            contrast_guidance(true, [255; 3], [0, 120, 215]),
            ("Low contrast 4.50:1. Recommended text: #000000.".to_owned(), true)
        );
        assert_eq!(
            contrast_guidance(true, [255; 3], [0, 0, 0]),
            ("Contrast 21.00:1. Recommended text: #FFFFFF.".to_owned(), false)
        );
        assert_eq!(
            contrast_guidance(false, [255; 3], [0, 0, 0]),
            ("Background hidden: use text that contrasts with the taskbar.".to_owned(), false)
        );
    }

    #[test]
    fn number_fields_only_accept_values_in_range() {
        let mut field = NumberField::new(60, 30, 3600);
        field.text = "12".to_owned();
        assert!(!field.accept_text());
        assert!(field.pending());
        field.text = "120".to_owned();
        assert!(field.accept_text());
        assert_eq!((field.value, field.pending()), (120, false));
        field.text = String::new();
        assert!(field.pending());
        field.set(99_999);
        assert_eq!((field.value, field.text.as_str()), (3600, "3600"));
    }

    #[test]
    fn the_form_round_trips_settings_and_tracks_edits() {
        let mut base = Settings::default();
        base.icon_metric = ICON_METRIC_DEVICE.to_owned();
        base.icon_device_id = "b".to_owned();
        base.critical_threshold = 40;
        base.low_threshold = 30;
        let devices = [device("b", "Mouse", "", Some(80), false)];
        let mut form = Form::new(base.clone(), true, &devices);
        assert_eq!((form.critical.value, form.low.value), (40, 40));
        let saved = form.candidate();
        assert_eq!(saved.icon_device_name, "Mouse");
        assert!(saved.start_with_windows);
        assert!(saved.editable_equals(&form.candidate()));

        form.text_color = [0, 0, 0];
        assert!(!saved.editable_equals(&form.candidate()));
        form.restore_defaults();
        let defaults = form.candidate();
        assert_eq!(defaults.icon_metric, ICON_METRIC_LOWEST);
        assert_eq!((defaults.critical_threshold, defaults.low_threshold, defaults.refresh_seconds), (15, 25, 60));
        assert_eq!(defaults.icon_device_id, "b");
        assert!(!defaults.start_with_windows);
    }
}
