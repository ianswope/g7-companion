//! The macOS menu dropdown: the reading and the last three hours, drawn in
//! the menu that opens when the status item is clicked. Windows keeps the
//! text-only menu. The geometry comes from `model::chart_content`, the same
//! rules as the Omarchy panel.

use std::cell::RefCell;

use g7::icon::{self, Rgb};
use g7::model::{self, Band, ChartContent};
use g7::poller::Snapshot;
use g7::store::Store;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBezierPath, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSMenu,
    NSMenuItem, NSStringDrawingDeprecated, NSStringDrawingOptions, NSView,
};
use objc2_foundation::{MainThreadMarker, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use tray_icon::menu::{ContextMenu, Menu};

const WIDTH: f64 = 320.0;
const PAD: f64 = 16.0;
const VALUE_H: f64 = 44.0;
const CHART_H: f64 = 90.0;
const AXIS_H: f64 = 16.0;

struct Rows {
    detail: f64,
    age: f64,
    chart: bool,
    status: f64,
    total: f64,
}

fn rows(content: &ChartContent) -> Rows {
    let detail = if content.detail.is_empty() {
        0.0
    } else if content.detail.chars().count() > 42 {
        34.0
    } else {
        18.0
    };
    let age = if content.age.is_empty() { 0.0 } else { 18.0 };
    let chart = content.show_chart();
    let status = if content.status.is_empty() { 0.0 } else { 36.0 };
    let mut total = 14.0 + VALUE_H + detail + age;
    if chart {
        total += 8.0 + CHART_H + AXIS_H;
    }
    if status > 0.0 {
        total += 8.0 + status;
    }
    total += 12.0;
    Rows { detail, age, chart, status, total }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect {
        origin: NSPoint { x, y },
        size: NSSize { width: w.max(0.0), height: h.max(0.0) },
    }
}

fn rgb(c: Rgb, alpha: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.0 as f64 / 255.0, c.1 as f64 / 255.0, c.2 as f64 / 255.0, alpha)
}

fn warn_color() -> Retained<NSColor> {
    rgb(icon::band_color(Some(Band::UrgentLow), false), 1.0)
}

fn headline_color(content: &ChartContent) -> Retained<NSColor> {
    if let Some(band) = content.headline_band {
        rgb(icon::band_color(Some(band), false), 1.0)
    } else if content.stale {
        NSColor::secondaryLabelColor()
    } else {
        NSColor::labelColor()
    }
}

fn dot_color(band: Band) -> Retained<NSColor> {
    match band {
        Band::InRange => NSColor::labelColor(),
        other => rgb(icon::band_color(Some(other), false), 1.0),
    }
}

/// `&NSFont` coerces to `&AnyObject` here, which a slice element will not do on its own.
fn as_any(obj: &AnyObject) -> &AnyObject {
    obj
}

fn text_attrs(font: &NSFont, color: &NSColor) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    let objects = [as_any(font), as_any(color)];
    NSDictionary::from_slices(&keys, &objects)
}

fn measure(text: &str, font: &NSFont, color: &NSColor, max_width: f64) -> NSRect {
    let s = NSString::from_str(text);
    let attrs = text_attrs(font, color);
    unsafe {
        s.boundingRectWithSize_options_attributes(
            NSSize { width: max_width, height: 200.0 },
            NSStringDrawingOptions::UsesLineFragmentOrigin,
            Some(&attrs),
        )
    }
}

fn draw_text(text: &str, area: NSRect, font: &NSFont, color: &NSColor) {
    let s = NSString::from_str(text);
    let attrs = text_attrs(font, color);
    unsafe {
        s.drawWithRect_options_attributes(
            area,
            NSStringDrawingOptions::UsesLineFragmentOrigin | NSStringDrawingOptions::TruncatesLastVisibleLine,
            Some(&attrs),
        );
    }
}

fn draw_chart(content: &ChartContent, x: f64, y: f64, w: f64) {
    let y_at = |unit: f64| y + (1.0 - unit.clamp(0.0, 1.0)) * CHART_H;
    let high = y_at(content.high_y);
    let low = y_at(content.low_y);
    NSColor::labelColor().colorWithAlphaComponent(0.08).setFill();
    NSBezierPath::bezierPathWithRect(rect(x, high, w, low - high)).fill();

    NSColor::separatorColor().setStroke();
    for line_y in [high, low] {
        let path = NSBezierPath::bezierPath();
        path.moveToPoint(NSPoint { x, y: line_y });
        path.lineToPoint(NSPoint { x: x + w, y: line_y });
        path.setLineWidth(1.0);
        path.stroke();
    }

    let radius = 2.5;
    for dot in &content.points {
        let cx = x + radius + dot.x.clamp(0.0, 1.0) * (w - 2.0 * radius);
        let cy = y_at(dot.y);
        dot_color(dot.band).setFill();
        NSBezierPath::bezierPathWithOvalInRect(rect(cx - radius, cy - radius, radius * 2.0, radius * 2.0)).fill();
    }
}

fn paint(content: &ChartContent, bounds: NSRect) {
    let rows = rows(content);
    let width = bounds.size.width;
    let text_w = (width - 2.0 * PAD).max(0.0);
    let label = NSColor::labelColor();
    let secondary = NSColor::secondaryLabelColor();
    let warned = warn_color();
    let mut y = 14.0;

    let value_font = NSFont::boldSystemFontOfSize(if content.headline.chars().count() <= 5 { 34.0 } else { 18.0 });
    let value_color = headline_color(content);
    let value_w = measure(&content.headline, &value_font, &value_color, text_w).size.width.min(text_w * 0.62);
    draw_text(&content.headline, rect(PAD, y, value_w.max(1.0), VALUE_H), &value_font, &value_color);
    if !content.trend.is_empty() {
        let trend_font = NSFont::systemFontOfSize(13.0);
        let trend_color: &NSColor = if content.stale { &secondary } else { &label };
        let tx = PAD + value_w + 10.0;
        draw_text(&content.trend, rect(tx, y + VALUE_H - 22.0, (width - PAD - tx).max(1.0), 18.0), &trend_font, trend_color);
    }
    y += VALUE_H;

    let caption = NSFont::systemFontOfSize(11.0);
    if rows.detail > 0.0 {
        draw_text(&content.detail, rect(PAD, y, text_w, rows.detail), &caption, &secondary);
        y += rows.detail;
    }
    if rows.age > 0.0 {
        let age_color: &NSColor = if content.age_warn { &warned } else { &secondary };
        draw_text(&content.age, rect(PAD, y, text_w, rows.age), &caption, age_color);
        y += rows.age;
    }
    if rows.chart {
        y += 8.0;
        draw_chart(content, PAD, y, text_w);
        y += CHART_H;
        let ago = if content.window_minutes % 60 == 0 {
            format!("{}h ago", content.window_minutes / 60)
        } else {
            format!("{}m ago", content.window_minutes)
        };
        draw_text(&ago, rect(PAD, y, text_w * 0.5, AXIS_H), &caption, &secondary);
        let now_w = measure("now", &caption, &secondary, 40.0).size.width;
        draw_text("now", rect(width - PAD - now_w, y, now_w.max(1.0), AXIS_H), &caption, &secondary);
        y += AXIS_H;
    }
    if rows.status > 0.0 {
        y += 8.0;
        let status_color: &NSColor = if content.status_warn { &warned } else { &secondary };
        draw_text(&content.status, rect(PAD, y, text_w, rows.status), &caption, status_color);
    }
}

struct ChartIvars {
    content: RefCell<ChartContent>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "G7CompanionChartView"]
    #[ivars = ChartIvars]
    struct ChartView;

    impl ChartView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _: NSRect) {
            let content = self.ivars().content.borrow();
            paint(&content, self.bounds());
        }
    }
);

impl ChartView {
    fn new(mtm: MainThreadMarker, content: ChartContent) -> Retained<Self> {
        let frame = rect(0.0, 0.0, WIDTH, rows(&content).total);
        let this = Self::alloc(mtm).set_ivars(ChartIvars { content: RefCell::new(content) });
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        view
    }
}

pub struct Slot {
    view: Retained<ChartView>,
    item: Retained<NSMenuItem>,
    menu: Retained<NSMenu>,
}

impl Slot {
    pub fn sync(&self, store: &Store, snap: &Snapshot) {
        let settings = store.load_settings();
        let content = model::chart_content(
            &snap.readings,
            model::now(),
            settings.unit,
            &settings.thresholds(),
            settings.stale_minutes(),
            snap.signed_in,
            snap.needs_login,
            &snap.account,
            &snap.message,
        );
        let height = rows(&content).total;
        let title = spoken(&content);
        *self.view.ivars().content.borrow_mut() = content;
        let width = self.view.frame().size.width.max(WIDTH);
        self.view.setFrameSize(NSSize { width, height });
        self.item.setTitle(&NSString::from_str(&title));
        self.menu.itemChanged(&self.item);
        self.view.setNeedsDisplay(true);
    }
}

fn spoken(content: &ChartContent) -> String {
    let mut parts = vec![content.headline.clone()];
    if !content.trend.is_empty() {
        parts.push(content.trend.clone());
    }
    if !content.age.is_empty() {
        parts.push(content.age.clone());
    }
    parts.join(", ")
}

/// Insert the chart as the first row of the status-item menu.
pub fn attach(menu: &Menu) -> Slot {
    let mtm = MainThreadMarker::new().expect("the menu chart is created on the main thread");
    let ns_menu = unsafe { Retained::retain(menu.ns_menu().cast::<NSMenu>()).expect("status menu") };
    let view = ChartView::new(mtm, model::chart_content(&[], model::now(), g7::model::Unit::Mgdl, &g7::model::Thresholds::sane(55, 70, 180), 10, false, false, "", ""));
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Glucose"),
            None,
            &NSString::from_str(""),
        )
    };
    // A custom view still draws when the item is disabled, and a disabled item
    // is not the blue selection row. AppKit would otherwise turn it back on.
    item.setEnabled(false);
    item.setView(Some(&*view));
    ns_menu.setAutoenablesItems(false);
    ns_menu.setMinimumWidth(WIDTH);
    ns_menu.insertItem_atIndex(&item, 0);
    Slot { view, item, menu: ns_menu }
}
