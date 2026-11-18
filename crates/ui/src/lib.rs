use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModuleTab {
    Income,
    Expenses,
    Savings,
    PaymentDocuments,
    Analysis,
    Directories,
}

impl ModuleTab {
    const ALL: [Self; 6] = [
        Self::Income,
        Self::Expenses,
        Self::Savings,
        Self::PaymentDocuments,
        Self::Analysis,
        Self::Directories,
    ];

    const fn title(self) -> &'static str {
        match self {
            Self::Income => "Доходы",
            Self::Expenses => "Расходы",
            Self::Savings => "Накопления",
            Self::PaymentDocuments => "Платёжные документы",
            Self::Analysis => "Анализ",
            Self::Directories => "Справочники",
        }
    }
}

#[derive(Default)]
struct FinanceApp {
    active_tab: Option<ModuleTab>,
}

impl eframe::App for FinanceApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("module_tabs").show(context, |ui| {
            ui.horizontal_wrapped(|ui| {
                for tab in ModuleTab::ALL {
                    ui.selectable_value(&mut self.active_tab, Some(tab), tab.title());
                }
            });
        });

        egui::CentralPanel::default().show(context, |ui| {
            ui.heading(self.active_tab.map_or("FinanceApp", ModuleTab::title));
            ui.separator();
            ui.label("Раздел подготовлен для следующего этапа разработки.");
        });
    }
}

/// Starts the native desktop window.
pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([1280.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "FinanceApp — управление финансами",
        options,
        Box::new(|_creation_context| Ok(Box::<FinanceApp>::default())),
    )
}