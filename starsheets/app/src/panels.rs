//! The side panels at the right of the sheet (conditional formatting, data validation, the chart
//! editor, named ranges, protected ranges, alternating colours, pivot tables): one shape for all of them, and the rule that only one
//! shows at a time (the one that just opened wins).

use egui::Ui;
use gridcraft_ui_egui::SheetApp;

use crate::altcolours::AltColoursPanel;
use crate::cfpanel::CfPanel;
use crate::chartpanel::ChartPanel;
use crate::dvpanel::DvPanel;
use crate::namespanel::NamesPanel;
use crate::pivotpanel::PivotPanel;
use crate::protectpanel::ProtectPanel;

/// What every side panel does each frame.
pub trait SidePanel {
    /// Takes GridCraft's request to open its own dialog for this, and opens the panel instead.
    fn intercept(&mut self, app: &mut SheetApp, ctx: &egui::Context);
    /// Whether this frame's `intercept` opened it.
    fn just_opened(&self) -> bool;
    fn close(&mut self, app: &mut SheetApp);
    fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui);
}

impl SidePanel for CfPanel {
    fn intercept(&mut self, app: &mut SheetApp, _ctx: &egui::Context) {
        let was = self.open;
        CfPanel::intercept(self, app);
        self.just_opened = self.open && !was;
    }
    fn just_opened(&self) -> bool {
        self.just_opened
    }
    fn close(&mut self, app: &mut SheetApp) {
        CfPanel::close(self, app);
    }
    fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        CfPanel::ui(self, app, ui);
    }
}

/// The panels whose `close` needs nothing from the app.
macro_rules! side_panel {
    ($t:ty, $intercept:expr) => {
        impl SidePanel for $t {
            fn intercept(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
                let f: fn(&mut $t, &mut SheetApp, &egui::Context) = $intercept;
                f(self, app, ctx);
            }
            fn just_opened(&self) -> bool {
                self.just_opened
            }
            fn close(&mut self, _app: &mut SheetApp) {
                <$t>::close(self);
            }
            fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
                <$t>::ui(self, app, ui);
            }
        }
    };
}

side_panel!(DvPanel, |p, app, ctx| p.intercept(app, ctx));
side_panel!(ChartPanel, |p, app, ctx| p.intercept(app, ctx));
side_panel!(NamesPanel, |p, app, _| p.intercept(app));
side_panel!(ProtectPanel, |p, app, ctx| p.intercept(app, ctx));
side_panel!(AltColoursPanel, |p, app, ctx| p.intercept(app, ctx));
side_panel!(PivotPanel, |p, app, ctx| p.intercept(app, ctx));

/// All the side panels.
#[derive(Default)]
pub struct SidePanels {
    pub cf: CfPanel,
    pub dv: DvPanel,
    pub chart: ChartPanel,
    pub names: NamesPanel,
    pub protect: ProtectPanel,
    pub alternating: AltColoursPanel,
    pub pivot: PivotPanel,
}

impl SidePanels {
    /// In the order they are drawn.
    fn all(&mut self) -> [&mut dyn SidePanel; 7] {
        [&mut self.cf, &mut self.dv, &mut self.chart, &mut self.names, &mut self.protect, &mut self.alternating, &mut self.pivot]
    }

    /// Opens a panel in place of GridCraft's dialog; if one opened, the others close. When two
    /// open in the same frame the chart editor wins, then named ranges, protected ranges,
    /// alternating colours, data validation, conditional formatting and the pivot table editor
    /// (it opens by itself when the active cell is in a pivot table).
    pub fn intercept(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        for p in self.all() {
            p.intercept(app, ctx);
        }
        const PRIORITY: [usize; 7] = [2, 3, 4, 5, 1, 0, 6];
        let mut panels = self.all();
        let Some(winner) = PRIORITY.into_iter().find(|&i| panels[i].just_opened()) else { return };
        for (i, p) in panels.iter_mut().enumerate() {
            if i != winner {
                p.close(app);
            }
        }
    }

    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        for p in self.all() {
            p.ui(app, ui);
        }
    }
}
