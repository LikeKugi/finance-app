use std::path::PathBuf;
use std::rc::Rc;

use finance_core::TransactionKind;
use finance_db::Database;

use super::analysis::AnalysisPage;
use super::directories::DirectoriesPage;
use super::payment_documents::PaymentDocumentsPage;
use super::savings::SavingsPage;
use super::settings::SettingsPage;
use super::transactions::TransactionsPage;
use super::transfers::TransfersPage;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

/// Вкладки приложения; порядок и названия как в прежнем интерфейсе.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PageTab {
    Income,
    Expenses,
    Savings,
    Transfers,
    PaymentDocuments,
    Analysis,
    Directories,
    Settings,
}

impl PageTab {
    pub(crate) const ALL: [Self; 8] = [
        Self::Income,
        Self::Expenses,
        Self::Savings,
        Self::Transfers,
        Self::PaymentDocuments,
        Self::Analysis,
        Self::Directories,
        Self::Settings,
    ];

    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Income => "Доходы",
            Self::Expenses => "Расходы",
            Self::Savings => "Накопления",
            Self::Transfers => "Переводы",
            Self::PaymentDocuments => "Платёжные карты",
            Self::Analysis => "Анализ",
            Self::Directories => "Справочники",
            Self::Settings => "Настройки",
        }
    }
}

struct AppShell {
    active: PageTab,
    income: Entity<TransactionsPage>,
    expenses: Entity<TransactionsPage>,
    directories: Entity<DirectoriesPage>,
    savings: Entity<SavingsPage>,
    transfers: Entity<TransfersPage>,
    payment_documents: Entity<PaymentDocumentsPage>,
    analysis: Entity<AnalysisPage>,
    settings: Entity<SettingsPage>,
}

impl Render for AppShell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active;
        let selected = PageTab::ALL
            .iter()
            .position(|tab| *tab == active)
            .unwrap_or(0);
        let tabs = TabBar::new("main-tabs")
            .selected_index(selected)
            .on_click(cx.listener(|this: &mut Self, index: &usize, window, cx| {
                if let Some(tab) = PageTab::ALL.get(*index) {
                    this.active = *tab;
                    // Справочники могли измениться: обновляем данные открываемой вкладки.
                    match tab {
                        PageTab::Income => this.income.update(cx, |p, cx| p.reload(window, cx)),
                        PageTab::Expenses => this.expenses.update(cx, |p, cx| p.reload(window, cx)),
                        PageTab::Directories => this.directories.update(cx, |p, cx| p.reload(cx)),
                        PageTab::Savings => this.savings.update(cx, |p, cx| p.reload(window, cx)),
                        PageTab::Transfers => {
                            this.transfers.update(cx, |p, cx| p.reload(window, cx))
                        }
                        PageTab::PaymentDocuments => this
                            .payment_documents
                            .update(cx, |p, cx| p.reload(window, cx)),
                        PageTab::Analysis => this.analysis.update(cx, |p, cx| p.reload(window, cx)),
                        _ => {}
                    }
                    cx.notify();
                }
            }))
            .children(PageTab::ALL.iter().map(|tab| Tab::new().label(tab.title())));

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(tabs)
            .child(div().flex_1().min_h_0().child(match active {
                PageTab::Income => self.income.clone().into_any_element(),
                PageTab::Expenses => self.expenses.clone().into_any_element(),
                PageTab::Directories => self.directories.clone().into_any_element(),
                PageTab::Savings => self.savings.clone().into_any_element(),
                PageTab::Transfers => self.transfers.clone().into_any_element(),
                PageTab::PaymentDocuments => self.payment_documents.clone().into_any_element(),
                PageTab::Analysis => self.analysis.clone().into_any_element(),
                PageTab::Settings => self.settings.clone().into_any_element(),
            }))
    }
}

pub fn run(database: Database, database_path: PathBuf, app_version: &'static str) {
    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            init(cx);
            let options = WindowOptions {
                window_min_size: Some(size(px(1280.0), px(720.0))),
                ..WindowOptions::default()
            };
            let database = Rc::new(database);
            open_window(options, cx, move |window, cx| {
                let income = cx.new(|cx| {
                    TransactionsPage::new(database.clone(), TransactionKind::Income, window, cx)
                });
                let expenses = cx.new(|cx| {
                    TransactionsPage::new(database.clone(), TransactionKind::Expense, window, cx)
                });
                let directories = cx.new(|cx| DirectoriesPage::new(database.clone(), cx));
                let savings = cx.new(|cx| SavingsPage::new(database.clone(), window, cx));
                let transfers = cx.new(|cx| TransfersPage::new(database.clone(), window, cx));
                let payment_documents =
                    cx.new(|cx| PaymentDocumentsPage::new(database.clone(), window, cx));
                let analysis = cx.new(|cx| AnalysisPage::new(database.clone(), window, cx));
                let settings = cx.new(|cx| {
                    SettingsPage::new(
                        database.clone(),
                        database_path.clone(),
                        app_version,
                        window,
                        cx,
                    )
                });
                cx.new(|_| AppShell {
                    active: PageTab::Income,
                    income,
                    expenses,
                    directories,
                    savings,
                    transfers,
                    payment_documents,
                    analysis,
                    settings,
                })
            })
            .expect("не удалось открыть окно");
        });
}

#[cfg(test)]
mod tests {
    use gpui_kit::assets::{Assets, IconName};
    use gpui_kit::AssetSource;

    #[test]
    fn bundled_plus_icon_is_available() {
        let icon_path = IconName::Plus.path();
        let icon = Assets
            .load(icon_path.as_ref())
            .expect("ошибка чтения встроенного значка Plus");

        assert!(icon.is_some(), "встроенный значок Plus отсутствует");
    }
}
