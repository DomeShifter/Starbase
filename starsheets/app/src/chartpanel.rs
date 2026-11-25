//! The chart editor, after Google Sheets: a side panel with Setup (chart type, stacking, data
//! range, X-axis, series) and Customise (chart style, titles, series, legend, gridlines) as
//! sections that open and close. Every change shows on the chart at once (each is one undo
//! step). It opens when a chart is inserted (Insert › Chart, the toolbar), double-clicked, or
//! chosen from its right-click menu, and follows the selected chart.

use egui::{Color32, RichText, Stroke, Ui, vec2};
use gridcraft_engine::model::{Chart, ChartKind, LegendPos, ObjKind, ObjRef, Trendline, TrendlineKind};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::{house, theme};

/// Asks for a new chart from the selection, with the editor open on it (Insert › Chart).
pub const REQUEST: &str = "starsheets_chart_new";
const PANEL_W: f32 = 320.0;

/// Google's chart types (label, type, subtype), then the extra kinds an Excel file can bring.
const TYPES: &[(&str, &str, &str)] = &[
    ("Line chart", "line", ""),
    ("Smooth line chart", "line", "smooth"),
    ("Line chart with points", "line", "markers"),
    ("Area chart", "area", ""),
    ("Stacked area chart", "area", "stacked"),
    ("100% stacked area chart", "area", "stacked100"),
    ("Column chart", "column", "clustered"),
    ("Stacked column chart", "column", "stacked"),
    ("100% stacked column chart", "column", "stacked100"),
    ("Bar chart", "bar", "clustered"),
    ("Stacked bar chart", "bar", "stacked"),
    ("100% stacked bar chart", "bar", "stacked100"),
    ("Pie chart", "pie", ""),
    ("Doughnut chart", "doughnut", ""),
    ("Scatter chart", "scatter", ""),
    ("Scatter chart with lines", "scatter", "lines"),
    ("Bubble chart", "bubble", ""),
    ("Combo chart", "combo", ""),
    ("Histogram chart", "histogram", ""),
    ("Radar chart", "radar", ""),
    ("Waterfall chart", "waterfall", ""),
    ("Treemap chart", "treemap", ""),
    ("Candlestick chart", "stock", ""),
    ("Funnel chart", "funnel", ""),
    ("Sunburst chart", "sunburst", ""),
    ("Box and whisker chart", "boxWhisker", ""),
];
/// Dividers in the type list: after the lines, areas, columns, bars, pies and scatters.
const TYPE_GROUPS: &[usize] = &[2, 5, 8, 11, 13, 16, 22];

/// Google's trendline types (label, the engine's `type`).
const TREND_TYPES: &[(&str, &str)] = &[
    ("Linear", "linear"),
    ("Exponential", "exponential"),
    ("Polynomial", "polynomial"),
    ("Logarithmic", "logarithmic"),
    ("Power series", "power"),
    ("Moving average", "movingAverage"),
];
/// Polynomial degrees and moving-average periods offered (Sheets offers the same ranges).
const DEGREES: &[&str] = &["2", "3", "4", "5", "6"];
const PERIODS: &[&str] = &["2", "3", "4", "5", "6", "7", "8", "9", "10"];
const TREND_LABELS: &[&str] = &["None", "Use equation"];

/// A trendline's settings as the editor shows them: type index, polynomial degree, moving-average
/// period, equation label, R².
#[derive(Clone, Copy, Debug, PartialEq)]
struct TrendChoice {
    kind: usize,
    degree: u8,
    period: u32,
    equation: bool,
    r_squared: bool,
}

impl TrendChoice {
    fn of(t: &Trendline) -> Self {
        let (kind, degree, period) = match t.kind {
            TrendlineKind::Linear => (0, 2, 2),
            TrendlineKind::Exponential => (1, 2, 2),
            TrendlineKind::Polynomial(d) => (2, d, 2),
            TrendlineKind::Logarithmic => (3, 2, 2),
            TrendlineKind::Power => (4, 2, 2),
            TrendlineKind::MovingAverage(n) => (5, 2, n),
        };
        TrendChoice { kind, degree, period, equation: t.display_equation, r_squared: t.display_r_squared }
    }

    /// `chart.addElement`'s parameters for this trendline on series `i`.
    fn params(&self, chart: u32, i: usize) -> serde_json::Value {
        json!({
            "chart": chart, "element": "trendline", "series": i, "on": true,
            "type": TREND_TYPES[self.kind].1, "order": self.degree, "period": self.period,
            "displayEquation": self.equation, "displayRSquared": self.r_squared,
        })
    }
}

const LEGEND: &[(&str, &str)] = &[("Bottom", "bottom"), ("Top", "top"), ("Left", "left"), ("Right", "right"), ("None", "none")];
/// Colour schemes: two fixed ones, then one per theme accent colour, named after the colour it
/// really is in this file's theme (to-do row 60: fixed Office names said "Grey", "Gold" and
/// "Light blue" for what the newer theme draws green, light blue and purple).
const SCHEMES: &[(&str, &str)] = &[("Colourful", "colorful"), ("Monochrome", "monochrome")];

/// Plain names for theme colours: the nearest of these is used.
const COLOUR_NAMES: &[(&str, u32)] = &[
    ("Blue", 0x4472C4),
    ("Dark blue", 0x1F3864),
    ("Light blue", 0x5B9BD5),
    ("Sky blue", 0x0F9ED5),
    ("Teal", 0x156082),
    ("Turquoise", 0x2AA198),
    ("Green", 0x4EA72E),
    ("Dark green", 0x196B24),
    ("Orange", 0xE97132),
    ("Gold", 0xFFC000),
    ("Yellow", 0xFFD966),
    ("Red", 0xC00000),
    ("Pink", 0xE91E63),
    ("Purple", 0xA02B93),
    ("Violet", 0x7030A0),
    ("Brown", 0x843C0C),
    ("Grey", 0xA5A5A5),
    ("Dark grey", 0x595959),
];

fn colour_name(rgb: u32) -> &'static str {
    let ch = |c: u32, s: u32| ((c >> s) & 0xFF) as i32;
    let dist = |a: u32, b: u32| (0..3).map(|k| (ch(a, k * 8) - ch(b, k * 8)).pow(2)).sum::<i32>();
    COLOUR_NAMES.iter().min_by_key(|(_, c)| dist(rgb, *c)).map_or("Colour", |(n, _)| n)
}

/// The scheme list for this workbook: (menu name, palette id).
fn schemes(app: &SheetApp) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = SCHEMES.iter().map(|(n, p)| (n.to_string(), p.to_string())).collect();
    let Some(d) = app.session.active() else { return out };
    let mut used: Vec<String> = Vec::new();
    for k in 1..=6u8 {
        let rgb = crate::cfpanel::rgb_of(&gridcraft_engine::model::Color::Theme(3 + k, 0), &d.wb.theme).unwrap_or(0);
        let mut name = colour_name(rgb).to_string();
        // Two accents with the same nearest name: say which is darker.
        if used.contains(&name) {
            name = format!("{name} {}", used.iter().filter(|u| u.starts_with(&name)).count() + 1);
        }
        used.push(name.clone());
        out.push((name, format!("accent{k}")));
    }
    out
}

const HOLES: &[(&str, u32)] = &[("None", 0), ("25%", 25), ("50%", 50), ("75%", 75)];
const TITLES: &[&str] = &["Chart title", "Horizontal axis title", "Vertical axis title"];

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Setup,
    Customise,
}

#[derive(Default)]
pub struct ChartPanel {
    /// The chart being edited (its id on the active sheet).
    pub chart: Option<u32>,
    /// True the frame the panel opened (so the other side panels close).
    pub just_opened: bool,
    tab: Option<Tab>,
    /// Which Customise sections are open.
    open: [bool; 5],
    title_which: usize,
    series_which: usize,
    /// Pie slices: every slice listed, not only the first SLICES_SHOWN.
    all_slices: bool,
    scheme: usize,
    /// The data range and X-axis boxes while they are being typed in.
    range_draft: Option<String>,
    x_draft: Option<String>,
}

fn kind_index(c: &Chart) -> usize {
    let smooth = c.series.iter().any(|s| s.smooth);
    let find = |t: &str, sub: &str| TYPES.iter().position(|x| x.1 == t && x.2 == sub).unwrap_or(0);
    match c.kind {
        ChartKind::Line if smooth => find("line", "smooth"),
        ChartKind::Line | ChartKind::LineStacked => find("line", ""),
        ChartKind::LineMarkers => find("line", "markers"),
        ChartKind::Area => find("area", ""),
        ChartKind::AreaStacked => find("area", "stacked"),
        ChartKind::AreaStacked100 => find("area", "stacked100"),
        ChartKind::ColumnClustered => find("column", "clustered"),
        ChartKind::ColumnStacked => find("column", "stacked"),
        ChartKind::ColumnStacked100 => find("column", "stacked100"),
        ChartKind::BarClustered => find("bar", "clustered"),
        ChartKind::BarStacked => find("bar", "stacked"),
        ChartKind::BarStacked100 => find("bar", "stacked100"),
        ChartKind::Pie => find("pie", ""),
        ChartKind::Doughnut => find("doughnut", ""),
        ChartKind::Scatter => find("scatter", ""),
        ChartKind::ScatterLines => find("scatter", "lines"),
        ChartKind::Bubble => find("bubble", ""),
        ChartKind::Combo => find("combo", ""),
        ChartKind::Histogram => find("histogram", ""),
        ChartKind::Radar => find("radar", ""),
        ChartKind::Waterfall => find("waterfall", ""),
        ChartKind::Treemap => find("treemap", ""),
        ChartKind::Stock => find("stock", ""),
        ChartKind::Funnel => find("funnel", ""),
        ChartKind::Sunburst => find("sunburst", ""),
        ChartKind::BoxWhisker => find("boxWhisker", ""),
    }
}

/// Column, bar and area charts can be stacked: their type and current stacking (0 none,
/// 1 standard, 2 100%).
fn stacking(c: &Chart) -> Option<(&'static str, usize)> {
    Some(match c.kind {
        ChartKind::ColumnClustered => ("column", 0),
        ChartKind::ColumnStacked => ("column", 1),
        ChartKind::ColumnStacked100 => ("column", 2),
        ChartKind::BarClustered => ("bar", 0),
        ChartKind::BarStacked => ("bar", 1),
        ChartKind::BarStacked100 => ("bar", 2),
        ChartKind::Area => ("area", 0),
        ChartKind::AreaStacked => ("area", 1),
        ChartKind::AreaStacked100 => ("area", 2),
        _ => return None,
    })
}

/// A series name as shown: the text of the cell it points at, else the name as written.
fn series_name(app: &SheetApp, name: Option<&str>, i: usize) -> String {
    let Some(n) = name.filter(|n| !n.is_empty()) else { return format!("Series {}", i + 1) };
    let shown = (|| {
        let d = app.session.active()?;
        let (sheet, cell) = n.rsplit_once('!').map_or((None, n), |(s, c)| (Some(s.trim_matches('\'')), c));
        let sh = match sheet {
            Some(s) => d.wb.sheets.get(d.wb.sheet_index(s)?)?.clone(),
            None => d.wb.sheets.get(d.wb.active_sheet())?.clone(),
        };
        let cell = gridcraft_engine::core::CellRef::parse(&cell.replace('$', ""))?;
        sh.value(cell).to_text().ok().filter(|t| !t.is_empty())
    })();
    shown.unwrap_or_else(|| n.trim_matches('"').to_string())
}

impl ChartPanel {
    pub fn close(&mut self) {
        self.chart = None;
        self.range_draft = None;
        self.x_draft = None;
    }

    fn open_on(&mut self, id: u32) {
        if self.chart != Some(id) {
            self.range_draft = None;
            self.x_draft = None;
            self.series_which = 0;
        }
        self.chart = Some(id);
        self.just_opened = true;
    }

    /// Takes over GridCraft's chart pane and Select Data dialog (double-click on a chart, its
    /// right-click menu) and Insert › Chart; follows the selected chart while open.
    pub fn intercept(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        self.just_opened = false;
        if ctx.data_mut(|m| m.remove_temp::<bool>(egui::Id::unique(REQUEST))).is_some() {
            let before: Vec<u32> = charts(app).iter().map(|c| c.id).collect();
            // GridCraft explains it when the selection can't be charted.
            app.run_or_alert("insert.chart", json!({}));
            if let Some(id) = charts(app).iter().map(|c| c.id).find(|id| !before.contains(id)) {
                app.selected_object = Some(ObjRef::new(ObjKind::Chart, id));
                self.tab = Some(Tab::Setup);
                self.open_on(id);
            }
        }
        if app.grid.pane.as_deref() == Some("formatChart") {
            app.grid.pane = None;
            if let Some(id) = app.selected_chart() {
                self.open_on(id);
            }
        }
        let dialog_chart = app.dialog.as_ref().filter(|d| d.name == "selectData").map(|d| d.values.get("chart").and_then(serde_json::Value::as_u64));
        if let Some(id) = dialog_chart {
            app.dialog = None;
            if let Some(id) = id.map(|i| i as u32).or_else(|| app.selected_chart()) {
                self.tab = Some(Tab::Setup);
                self.open_on(id);
            }
        }
        // Follow the selected chart; close when ours is gone (deleted, another sheet).
        if self.chart.is_some()
            && let Some(id) = app.selected_chart()
        {
            self.open_on_quiet(id);
        }
        if let Some(id) = self.chart
            && !charts(app).iter().any(|c| c.id == id)
        {
            self.close();
        }
    }

    fn open_on_quiet(&mut self, id: u32) {
        if self.chart != Some(id) {
            self.range_draft = None;
            self.x_draft = None;
            self.series_which = 0;
            self.chart = Some(id);
        }
    }

    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let Some(id) = self.chart else { return };
        let Some(chart) = charts(app).into_iter().find(|c| c.id == id) else { return };
        egui::Panel::right("starsheets_chart_editor")
            .exact_size(PANEL_W + 32.0 + crate::house::SCROLL_ROOM)
            .resizable(false)
            .frame(house::panel_frame())
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Chart editor").size(16.0).color(theme::TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if house::close_button(ui) {
                            self.close();
                        }
                    });
                });
                if self.chart.is_none() {
                    return;
                }
                let tab = *self.tab.get_or_insert(Tab::Setup);
                ui.horizontal(|ui| {
                    for (t, label) in [(Tab::Setup, "Setup"), (Tab::Customise, "Customise")] {
                        let on = tab == t;
                        let colour = if on { house::GREEN } else { theme::TEXT_DIM };
                        let r = ui.add(
                            egui::Button::new(RichText::new(label).size(house::BODY).color(colour))
                                .frame(false)
                                .min_size(vec2(PANEL_W / 2.0 - 4.0, 36.0)),
                        );
                        if on {
                            ui.painter().hline(r.rect.x_range().shrink(8.0), r.rect.bottom(), Stroke::new(3.0, house::GREEN));
                        }
                        if r.clicked() {
                            self.tab = Some(t);
                        }
                    }
                });
                ui.add_space(house::S);
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(crate::house::panel_scroll_height(ui)).show(ui, |ui| {
                    // White fields inside the scrolling part only: the scroll handle takes the
                    // fill of the panel around it.
                    house::form_style(ui);
                    ui.set_width(PANEL_W);
                    match tab {
                        Tab::Setup => self.setup(app, ui, &chart),
                        Tab::Customise => self.customise(app, ui, &chart),
                    }
                });
            });
    }

    fn setup(&mut self, app: &mut SheetApp, ui: &mut Ui, c: &Chart) {
        let id = c.id;
        house::label(ui, "Chart type", "");
        let labels: Vec<&str> = TYPES.iter().map(|t| t.0).collect();
        let mut k = kind_index(c);
        if house::select(ui, "chart_type", PANEL_W, &mut k, &labels, TYPE_GROUPS) {
            set_type(app, c, k);
        }
        if let Some((t, now)) = stacking(c) {
            house::gap(ui);
            house::label(ui, "Stacking", "");
            let mut s = now;
            if house::select(ui, "chart_stacking", PANEL_W, &mut s, &["None", "Standard", "100%"], &[]) {
                let sub = ["clustered", "stacked", "stacked100"][s];
                let sub = if t == "area" && s == 0 { "" } else { sub };
                app.run_or_alert("chart.set", json!({ "chart": id, "type": t, "subtype": sub }));
            }
        }
        house::gap(ui);
        house::label(ui, "Data range", "");
        let source = plain_range(app, c.source.as_deref().unwrap_or_default());
        let draft = self.range_draft.get_or_insert_with(|| source.clone());
        let r = ui.add(egui::TextEdit::singleline(draft).desired_width(PANEL_W).margin(vec2(house::M, house::S + 1.0)).hint_text("e.g. A1:D10"));
        if r.lost_focus() || (r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
            let want = draft.trim().to_string();
            if !want.is_empty() && want != source {
                app.run_or_alert("chart.selectData", json!({ "chart": id, "range": want }));
            }
            self.range_draft = None;
        } else if !r.has_focus() && *draft != source {
            self.range_draft = None;
        }
        house::gap(ui);
        house::label(ui, "X-axis", "The labels along the bottom (a range, or empty for none).");
        let cats = plain_range(app, c.series.first().and_then(|s| s.categories.as_deref()).unwrap_or_default());
        let draft = self.x_draft.get_or_insert_with(|| cats.clone());
        let r = ui.add(egui::TextEdit::singleline(draft).desired_width(PANEL_W).margin(vec2(house::M, house::S + 1.0)).hint_text("e.g. A2:A10"));
        if r.lost_focus() || (r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
            let want = draft.trim().to_string();
            if want != cats {
                app.run_or_alert("chart.selectData", json!({ "chart": id, "categories": want }));
            }
            self.x_draft = None;
        } else if !r.has_focus() && *draft != cats {
            self.x_draft = None;
        }
        house::gap(ui);
        house::label(ui, "Series", "");
        let mut remove = None;
        for (i, s) in c.series.iter().enumerate() {
            ui.horizontal(|ui| {
                let colour = series_colour(app, c, i);
                let (dot, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 6.0, colour);
                ui.label(RichText::new(series_name(app, s.name.as_deref(), i)).size(house::BODY).color(theme::TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if c.series.len() > 1 && ui.add(delete_button()).on_hover_text("Remove this series").clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove {
            app.run_or_alert("chart.selectData", json!({ "chart": id, "removeSeries": [i] }));
        }
        house::gap(ui);
        let mut by_rows = c.by_rows;
        if ui.checkbox(&mut by_rows, "Switch rows/columns").changed() {
            app.run_or_alert("chart.switchRowColumn", json!({ "chart": id }));
        }
    }

    fn customise(&mut self, app: &mut SheetApp, ui: &mut Ui, c: &Chart) {
        let id = c.id;
        let pie = matches!(c.kind, ChartKind::Pie | ChartKind::Doughnut);
        // Chart style.
        if section(ui, "Chart style", &mut self.open[0]) {
            house::label(ui, "Colours", "");
            let list = schemes(app);
            let names: Vec<&str> = list.iter().map(|s| s.0.as_str()).collect();
            if house::select(ui, "chart_scheme", PANEL_W, &mut self.scheme, &names, &[1]) {
                app.run_or_alert("chart.changeColors", json!({ "chart": id, "palette": list[self.scheme].1 }));
            }
            if matches!(c.kind, ChartKind::Line | ChartKind::LineMarkers | ChartKind::ScatterLines) {
                let mut smooth = c.series.iter().any(|s| s.smooth);
                if ui.checkbox(&mut smooth, "Smooth").changed() {
                    app.run_or_alert("chart.addElement", json!({ "chart": id, "element": "smooth", "on": smooth }));
                }
            }
            if pie {
                house::label(ui, "Doughnut hole", "");
                let now =
                    if c.kind == ChartKind::Pie { 0 } else { HOLES.iter().rposition(|h| h.1 > 0 && u32::from(c.hole_size) >= h.1).unwrap_or(2) };
                let mut k = now;
                if house::select(ui, "chart_hole", PANEL_W, &mut k, &HOLES.iter().map(|h| h.0).collect::<Vec<_>>(), &[]) {
                    if HOLES[k].1 == 0 {
                        app.run_or_alert("chart.set", json!({ "chart": id, "type": "pie" }));
                    } else {
                        app.run_or_alert("chart.set", json!({ "chart": id, "type": "doughnut", "holeSize": HOLES[k].1 }));
                    }
                }
                house::label(ui, "Rotation", "Where the first slice starts, in degrees clockwise.");
                let angles = ["0°", "45°", "90°", "135°", "180°", "225°", "270°", "315°"];
                let mut a = usize::from(c.first_slice_angle / 45).min(7);
                if house::select(ui, "chart_angle", PANEL_W, &mut a, &angles, &[]) {
                    app.run_or_alert("chart.set", json!({ "chart": id, "firstSliceAngle": a * 45 }));
                }
            }
        }
        // Chart and axis titles.
        if section(ui, "Chart & axis titles", &mut self.open[1]) {
            let which: &[&str] = if pie { &TITLES[..1] } else { TITLES };
            self.title_which = self.title_which.min(which.len() - 1);
            house::select(ui, "chart_title_which", PANEL_W, &mut self.title_which, which, &[]);
            let (key, now) = match self.title_which {
                1 => ("xTitle", c.x_title.clone()),
                2 => ("yTitle", c.y_title.clone()),
                _ => ("title", c.title.clone()),
            };
            house::label(ui, "Title text", "");
            let mut text = now.clone().unwrap_or_default();
            let r =
                ui.add(egui::TextEdit::singleline(&mut text).desired_width(PANEL_W).margin(vec2(house::M, house::S + 1.0)).hint_text("Type a title"));
            if r.changed() {
                // Live, as in Sheets; an emptied title is removed.
                let v = if text.is_empty() { serde_json::Value::Null } else { json!(text) };
                app.run_or_alert("chart.set", json!({ "chart": id, key: v }));
            }
        }
        // Pie and doughnut: each slice's colour, as Sheets' "Pie slice" section.
        if pie && section(ui, "Pie slices", &mut self.open[2]) {
            let Some(d) = app.session.active() else { return };
            let data = gridcraft_chart::resolve(&d.wb, d.wb.active_sheet(), c);
            let names: Vec<String> = data.categories.iter().take(200).cloned().collect();
            // Every slice on its own row (colour box + name), so several can be changed without
            // picking them one at a time from a list (to-do row 62). Long lists show the first
            // SLICES_SHOWN with "Show all" below, to keep the panel calm.
            if !names.is_empty() {
                let colour_of = |j: usize| {
                    let rgba = if j == 0 { data.series.first().map(|s| s.color) } else { data.palette.get(j % data.palette.len().max(1)).copied() };
                    rgba.map(|[r, g, b, _]| u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
                };
                house::label(ui, "Slice colours", "Click a colour box to change that slice.");
                let shown = if self.all_slices { names.len() } else { names.len().min(SLICES_SHOWN) };
                let mut pick = None;
                for (j, name) in names.iter().enumerate().take(shown) {
                    ui.horizontal(|ui| {
                        if let Some(rgb) = slice_swatch(ui, j, colour_of(j)) {
                            pick = Some((j, rgb));
                        }
                        ui.label(RichText::new(name).size(house::BODY).color(theme::TEXT));
                    });
                }
                if names.len() > SLICES_SHOWN {
                    let text = if self.all_slices { "Show fewer".to_string() } else { format!("Show all {} slices", names.len()) };
                    if ui.add(house::link(&text)).clicked() {
                        self.all_slices = !self.all_slices;
                    }
                }
                if let Some((j, rgb)) = pick {
                    app.run_or_alert("chart.formatSelection", json!({ "chart": id, "point": j, "color": format!("#{rgb:06X}") }));
                }
            }
            let mut labels_on = c.data_labels;
            if ui.checkbox(&mut labels_on, "Data labels").changed() {
                app.run_or_alert("chart.set", json!({ "chart": id, "dataLabels": labels_on }));
            }
        }
        // Series.
        if !pie && section(ui, "Series", &mut self.open[2]) {
            let mut names = vec!["Apply to all series".to_string()];
            names.extend(c.series.iter().enumerate().map(|(i, s)| series_name(app, s.name.as_deref(), i)));
            let labels: Vec<&str> = names.iter().map(String::as_str).collect();
            self.series_which = self.series_which.min(labels.len() - 1);
            house::select(ui, "chart_series_which", PANEL_W, &mut self.series_which, &labels, &[0]);
            let targets: Vec<usize> = if self.series_which == 0 { (0..c.series.len()).collect() } else { vec![self.series_which - 1] };
            house::label(ui, "Colour", "");
            let now = targets.first().map(|i| series_colour(app, c, *i)).map(|c| u32::from(c.r()) << 16 | u32::from(c.g()) << 8 | u32::from(c.b()));
            if let Some(rgb) = colour_button(ui, "chart_series_colour", now) {
                for i in &targets {
                    app.run_or_alert("chart.formatSelection", json!({ "chart": id, "series": i, "color": format!("#{rgb:06X}") }));
                }
            }
            if !pie {
                let mut trend = targets.iter().all(|i| c.series.get(*i).is_some_and(|s| !s.trendlines.is_empty()));
                if ui.checkbox(&mut trend, "Trendline").changed() {
                    for i in &targets {
                        app.run_or_alert("chart.addElement", json!({ "chart": id, "element": "trendline", "series": i, "on": trend }));
                    }
                }
                let now = targets.iter().find_map(|i| c.series.get(*i).and_then(|s| s.trendlines.first())).map(TrendChoice::of);
                if let (true, Some(now)) = (trend, now) {
                    trendline_options(app, ui, id, &targets, now);
                }
            }
            let mut labels_on = c.data_labels;
            if ui.checkbox(&mut labels_on, "Data labels").changed() {
                app.run_or_alert("chart.set", json!({ "chart": id, "dataLabels": labels_on }));
            }
        }
        // Legend.
        if section(ui, "Legend", &mut self.open[3]) {
            house::label(ui, "Position", "");
            let now = match c.legend {
                LegendPos::Bottom => 0,
                LegendPos::Top => 1,
                LegendPos::Left => 2,
                LegendPos::Right => 3,
                LegendPos::None => 4,
            };
            let mut k = now;
            if house::select(ui, "chart_legend", PANEL_W, &mut k, &LEGEND.iter().map(|l| l.0).collect::<Vec<_>>(), &[3]) {
                app.run_or_alert("chart.set", json!({ "chart": id, "legend": LEGEND[k].1 }));
            }
        }
        // Gridlines.
        if !pie && section(ui, "Gridlines and ticks", &mut self.open[4]) {
            let mut on = c.gridlines;
            if ui.checkbox(&mut on, "Major gridlines").changed() {
                app.run_or_alert("chart.set", json!({ "chart": id, "gridlines": on }));
            }
        }
    }
}

/// Sheets' trendline options under the Trendline tick: type, degree or period, label, R².
/// A change replaces the series' trendline (the engine adds one at a time).
fn trendline_options(app: &mut SheetApp, ui: &mut Ui, id: u32, targets: &[usize], now: TrendChoice) {
    let mut want = now;
    house::label(ui, "Type", "");
    let types: Vec<&str> = TREND_TYPES.iter().map(|t| t.0).collect();
    house::select(ui, "chart_trend_type", PANEL_W, &mut want.kind, &types, &[]);
    if want.kind == 2 {
        house::label(ui, "Polynomial degree", "");
        let mut k = usize::from(want.degree.clamp(2, 6) - 2);
        if house::select(ui, "chart_trend_degree", PANEL_W, &mut k, DEGREES, &[]) {
            want.degree = k as u8 + 2;
        }
    }
    if want.kind == 5 {
        house::label(ui, "Average of", "How many points each average covers.");
        let mut k = (want.period.clamp(2, 10) - 2) as usize;
        if house::select(ui, "chart_trend_period", PANEL_W, &mut k, PERIODS, &[]) {
            want.period = k as u32 + 2;
        }
    }
    house::label(ui, "Label", "");
    let mut k = usize::from(want.equation);
    if house::select(ui, "chart_trend_label", PANEL_W, &mut k, TREND_LABELS, &[]) {
        want.equation = k == 1;
    }
    // Moving averages have no equation or R² (as in Sheets).
    if want.kind != 5 {
        ui.checkbox(&mut want.r_squared, "Show R²");
    }
    if want != now {
        for i in targets {
            app.run_or_alert("chart.addElement", json!({ "chart": id, "element": "trendline", "series": i, "on": false }));
            app.run_or_alert("chart.addElement", want.params(id, *i));
        }
    }
}

/// How many pie slices are listed before "Show all".
const SLICES_SHOWN: usize = 12;

/// A small colour box for one pie slice; the colour picked, if any.
fn slice_swatch(ui: &mut Ui, j: usize, now: Option<u32>) -> Option<u32> {
    let swatch = egui::Button::new("")
        .fill(now.map_or(Color32::WHITE, |rgb| Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)))
        .stroke(Stroke::new(1.0, theme::BOX))
        .corner_radius(f32::from(house::FIELD_RADIUS))
        .min_size(vec2(32.0, 24.0));
    let r = ui.add(swatch).on_hover_text("Slice colour");
    let mut picked = None;
    egui::Popup::menu(&r).id(egui::Id::unique(("chart_slice_colour", j))).show(|ui| {
        if let Some(crate::colourpicker::Choice::Colour(rgb)) = crate::colourpicker::palette(ui, now, false, false) {
            picked = Some(rgb);
        }
    });
    picked
}

/// A colour box that opens the house colour menu; the colour picked, if any.
fn colour_button(ui: &mut Ui, id: &str, now: Option<u32>) -> Option<u32> {
    let swatch = egui::Button::new(RichText::new("   ").size(house::BODY))
        .fill(now.map_or(Color32::WHITE, |rgb| Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)))
        .stroke(Stroke::new(1.0, theme::BOX))
        .corner_radius(f32::from(house::FIELD_RADIUS))
        .min_size(vec2(64.0, house::FIELD_HEIGHT));
    let r = ui.add(swatch).on_hover_text("Colour");
    let mut picked = None;
    egui::Popup::menu(&r).id(egui::Id::unique(id)).show(|ui| {
        if let Some(crate::colourpicker::Choice::Colour(rgb)) = crate::colourpicker::palette(ui, now, false, false) {
            picked = Some(rgb);
        }
    });
    picked
}

/// A Customise section header (a full-width row with ▾/▴ and a line under it); true when open.
fn section(ui: &mut Ui, title: &str, open: &mut bool) -> bool {
    let arrow = if *open { "▴" } else { "▾" };
    let r = ui.add(
        egui::Button::new(RichText::new(title).size(house::BODY).strong().color(theme::TEXT))
            .right_text(RichText::new(arrow).size(house::BODY).color(theme::TEXT_DIM))
            .frame(false)
            .min_size(vec2(PANEL_W, 44.0)),
    );
    ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, theme::DIVIDER));
    if r.clicked() {
        *open = !*open;
    }
    if *open {
        ui.add_space(house::S);
    }
    *open
}

fn delete_button() -> egui::Button<'static> {
    match crate::icons::icon("delete") {
        Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(18.0, 18.0))).frame(false),
        None => egui::Button::new("✕").frame(false),
    }
}

fn charts(app: &SheetApp) -> Vec<Chart> {
    app.session.active().and_then(|d| d.wb.active()).map(|sh| sh.charts.clone()).unwrap_or_default()
}

/// The colour a series is drawn in: its own, else GridCraft's palette for the workbook's theme.
fn series_colour(app: &SheetApp, c: &Chart, i: usize) -> Color32 {
    let Some(d) = app.session.active() else { return theme::BLUE };
    let own = c.series.get(i).and_then(|s| s.color.as_ref()).and_then(|col| crate::cfpanel::rgb_of(col, &d.wb.theme));
    match own {
        Some(rgb) => Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8),
        None => {
            let [r, g, b, _] = gridcraft_chart::series_color(&d.wb.theme, i);
            Color32::from_rgb(r, g, b)
        }
    }
}

/// A range as Sheets shows it: no `$`, and no sheet name when it's the active sheet.
fn plain_range(app: &SheetApp, r: &str) -> String {
    let r = r.replace('$', "");
    let active = app.session.active().and_then(|d| d.wb.active()).map(|sh| sh.name.clone()).unwrap_or_default();
    match r.rsplit_once('!') {
        Some((sheet, rest)) if sheet.trim_matches('\'') == active => rest.to_string(),
        _ => r,
    }
}

fn set_type(app: &mut SheetApp, c: &Chart, k: usize) {
    let (_, t, sub) = TYPES[k];
    let id = c.id;
    let smooth_now = c.series.iter().any(|s| s.smooth);
    if sub == "smooth" {
        app.run_or_alert("chart.set", json!({ "chart": id, "type": "line" }));
        app.run_or_alert("chart.addElement", json!({ "chart": id, "element": "smooth", "on": true }));
        return;
    }
    app.run_or_alert("chart.set", json!({ "chart": id, "type": t, "subtype": sub }));
    if smooth_now && t == "line" {
        app.run_or_alert("chart.addElement", json!({ "chart": id, "element": "smooth", "on": false }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_chart_kind_has_a_place_in_the_type_list() {
        use gridcraft_engine::cmd::insert::chart_kind;
        for (label, t, sub) in TYPES {
            let sub = if *sub == "smooth" { "" } else { sub };
            let kind = chart_kind(t, sub);
            let c = Chart { kind, ..test_chart() };
            let back = TYPES[kind_index(&c)];
            assert_eq!(chart_kind(back.1, if back.2 == "smooth" { "" } else { back.2 }), kind, "{label}");
        }
    }

    #[test]
    fn plain_ranges_from_the_editor_are_accepted() {
        let mut s = gridcraft_engine::Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (c, v) in [("A1", "x"), ("B1", "y"), ("C1", "z"), ("A2", "a"), ("B2", "1"), ("C2", "4"), ("A3", "b"), ("B3", "2"), ("C3", "5")] {
            s.execute("cell.set", json!({ "cell": c, "input": v })).unwrap();
        }
        s.execute("insert.chart", json!({ "range": "A1:B3" })).unwrap();
        let id = s.active().unwrap().wb.active().unwrap().charts[0].id;
        s.execute("chart.selectData", json!({ "chart": id, "range": "A1:C3" })).unwrap();
        assert_eq!(s.active().unwrap().wb.active().unwrap().charts[0].series.len(), 2);
        s.execute("chart.selectData", json!({ "chart": id, "categories": "A2:A3" })).unwrap();
        s.execute("chart.set", json!({ "chart": id, "title": null })).unwrap();
        assert_eq!(s.active().unwrap().wb.active().unwrap().charts[0].title, None);
    }

    #[test]
    fn a_pie_slice_keeps_its_colour_through_xlsx() {
        let mut s = gridcraft_engine::Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (c, v) in [("A1", "x"), ("B1", "y"), ("A2", "a"), ("B2", "1"), ("A3", "b"), ("B3", "2"), ("A4", "c"), ("B4", "3")] {
            s.execute("cell.set", json!({ "cell": c, "input": v })).unwrap();
        }
        s.execute("insert.chart", json!({ "range": "A1:B4", "type": "pie" })).unwrap();
        let id = s.active().unwrap().wb.active().unwrap().charts[0].id;
        s.execute("chart.formatSelection", json!({ "chart": id, "point": 2, "color": "#FF0000" })).unwrap();
        let wb = &s.active().unwrap().wb;
        let bytes = gridcraft_engine::io::save_bytes(wb, "t.xlsx").unwrap();
        let (back, _) = gridcraft_engine::io::open_bytes("t.xlsx", &bytes).unwrap();
        let c = &back.sheets[0].charts[0];
        assert_eq!(c.point_colors.get(2).copied().flatten().and_then(|c| c.resolve(&back.theme)), Some([255, 0, 0]));
        let data = gridcraft_chart::resolve(&back, 0, c);
        assert_eq!(data.palette[2], [255, 0, 0, 255]);
        // A colour scheme colours every slice.
        s.execute("chart.changeColors", json!({ "chart": id, "palette": "monochrome" })).unwrap();
        assert_eq!(s.active().unwrap().wb.active().unwrap().charts[0].point_colors.len(), 3);
    }

    #[test]
    fn trendline_options_round_trip() {
        let mut s = gridcraft_engine::Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (c, v) in [("A1", "x"), ("B1", "y"), ("A2", "1"), ("B2", "1"), ("A3", "2"), ("B3", "4"), ("A4", "3"), ("B4", "9")] {
            s.execute("cell.set", json!({ "cell": c, "input": v })).unwrap();
        }
        s.execute("insert.chart", json!({ "range": "A1:B4", "type": "scatter" })).unwrap();
        let id = s.active().unwrap().wb.active().unwrap().charts[0].id;
        let want = TrendChoice { kind: 2, degree: 3, period: 2, equation: true, r_squared: true };
        s.execute("chart.addElement", want.params(id, 0)).unwrap();
        let t = s.active().unwrap().wb.active().unwrap().charts[0].series[0].trendlines[0].clone();
        assert_eq!(TrendChoice::of(&t), want);
        let avg = TrendChoice { kind: 5, degree: 2, period: 4, equation: false, r_squared: false };
        s.execute("chart.addElement", json!({ "chart": id, "element": "trendline", "series": 0, "on": false })).unwrap();
        s.execute("chart.addElement", avg.params(id, 0)).unwrap();
        let series = &s.active().unwrap().wb.active().unwrap().charts[0].series[0];
        assert_eq!(series.trendlines.len(), 1, "a change replaces the trendline");
        assert_eq!(TrendChoice::of(&series.trendlines[0]), avg);
    }

    fn test_chart() -> Chart {
        let mut s = gridcraft_engine::Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (c, v) in [("A1", "x"), ("B1", "y"), ("A2", "a"), ("B2", "1"), ("A3", "b"), ("B3", "2")] {
            s.execute("cell.set", json!({ "cell": c, "input": v })).unwrap();
        }
        s.execute("insert.chart", json!({ "range": "A1:B3" })).unwrap();
        s.active().unwrap().wb.active().unwrap().charts[0].clone()
    }
}
