use std::path::PathBuf;
use std::rc::Rc;

use finance_core::TransactionKind;
use finance_db::Database;

use super::analysis::AnalysisPage;
use super::directories::DirectoriesPage;
use super::home::{
    all_page_links, described_sections, home_action_tab, quick_actions, HomeAction, HomeLink,
};
use super::payment_documents::PaymentDocumentsPage;
use super::savings::SavingsPage;
use super::settings::SettingsPage;
use super::transactions::TransactionsPage;
use super::transfers::{can_create_transfer, TransfersPage};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _};
use gpui_kit::*;

/// Вкладки приложения; порядок и названия как в прежнем интерфейсе.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PageTab {
    Home,
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
    pub(crate) const ALL: [Self; 9] = [
        Self::Home,
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
            Self::Home => "Главная",
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
    database: Rc<Database>,
    transfer_available: bool,
    home_message: Option<String>,
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

impl AppShell {
    /// Проверка счетов для быстрого действия «Добавить перевод»: вызывается
    /// при открытии «Главной», но не на кадре отрисовки.
    fn refresh_transfer_availability(&mut self) {
        match (
            finance_db::list_payment_documents(&self.database, None, None, None),
            finance_db::list_savings(&self.database, None, None, ""),
        ) {
            (Ok(cards), Ok(savings)) => {
                self.transfer_available = can_create_transfer(&cards, &savings);
                self.home_message = None;
            }
            (Err(error), _) | (_, Err(error)) => {
                self.home_message = Some(format!(
                    "Не удалось проверить доступность переводов: {error}"
                ));
            }
        }
    }

    fn activate(&mut self, tab: PageTab, window: &mut Window, cx: &mut Context<Self>) {
        self.active = tab;
        match tab {
            PageTab::Home => self.refresh_transfer_availability(),
            PageTab::Income => self.income.update(cx, |page, cx| page.reload(window, cx)),
            PageTab::Expenses => self.expenses.update(cx, |page, cx| page.reload(window, cx)),
            PageTab::Directories => self.directories.update(cx, |page, cx| page.reload(cx)),
            PageTab::Savings => self.savings.update(cx, |page, cx| page.reload(window, cx)),
            PageTab::Transfers => self
                .transfers
                .update(cx, |page, cx| page.reload(window, cx)),
            PageTab::PaymentDocuments => self
                .payment_documents
                .update(cx, |page, cx| page.reload(window, cx)),
            PageTab::Analysis => self.analysis.update(cx, |page, cx| page.reload(window, cx)),
            PageTab::Settings => {}
        }
    }

    fn run_home_action(&mut self, action: HomeAction, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(home_action_tab(action), window, cx);
        match action {
            HomeAction::NewIncome => {
                let income = self.income.clone();
                income.update(cx, |page, cx| page.create_new(window, cx));
            }
            HomeAction::NewExpense => {
                let expenses = self.expenses.clone();
                expenses.update(cx, |page, cx| page.create_new(window, cx));
            }
            HomeAction::NewTransfer if self.transfer_available => {
                let transfers = self.transfers.clone();
                transfers.update(cx, |page, cx| page.create_new(window, cx));
            }
            _ => {}
        }
        cx.notify();
    }

    fn render_home_section<const N: usize>(
        &self,
        title: &'static str,
        links: [HomeLink; N],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let description_color = cx.theme().muted_foreground;
        let border = cx.theme().border;
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(18.0))
                    .font_weight(FontWeight::BOLD)
                    .child(title),
            )
            .child(div().flex().flex_wrap().gap_3().children(links.map(|link| {
                let action = link.action;
                div()
                    .w(px(360.0))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .border_1()
                    .border_color(border)
                    .rounded_md()
                    .child(
                        Button::new(format!("home-link-{}", link.title))
                            .link()
                            .label(link.title)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run_home_action(action, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(description_color)
                            .child(link.description),
                    )
            })))
            .into_any_element()
    }

    fn render_home(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let transfer_available = self.transfer_available;
        let message = self.home_message.clone();
        let mut root = div()
            .id("home-page")
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .overflow_y_scroll();
        root = root.child(
            div()
                .text_size(px(24.0))
                .font_weight(FontWeight::BOLD)
                .child("Главная"),
        );
        if let Some(message) = message {
            root = root.child(div().text_color(cx.theme().danger).child(message));
        }
        root.child(
            div()
                .text_size(px(18.0))
                .font_weight(FontWeight::BOLD)
                .child("Быстрые действия"),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_3()
                .children(quick_actions().map(|action| {
                    let selected = action.action;
                    Button::new(format!("home-action-{}", action.label))
                        .primary()
                        .large()
                        .label(action.label)
                        .icon(IconName::Plus)
                        .disabled(selected == HomeAction::NewTransfer && !transfer_available)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_home_action(selected, window, cx);
                        }))
                })),
        )
        .child(self.render_home_section("Разделы", described_sections(), cx))
        .child(self.render_home_section("Все страницы", all_page_links(), cx))
        .into_any_element()
    }
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
            .menu(true)
            .on_click(cx.listener(|this: &mut Self, index: &usize, window, cx| {
                if let Some(tab) = PageTab::ALL.get(*index) {
                    // Справочники могли измениться: обновляем данные открываемой вкладки.
                    this.activate(*tab, window, cx);
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
                PageTab::Home => self.render_home(cx),
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
                let transfer_available = matches!(
                    (
                        finance_db::list_payment_documents(&database, None, None, None),
                        finance_db::list_savings(&database, None, None, ""),
                    ),
                    (Ok(cards), Ok(savings)) if can_create_transfer(&cards, &savings)
                );
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
                    database: database.clone(),
                    transfer_available,
                    home_message: None,
                    active: PageTab::Home,
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

    use super::PageTab;

    #[test]
    fn home_is_the_first_tab_and_has_a_russian_title() {
        assert_eq!(PageTab::ALL.len(), 9);
        assert_eq!(PageTab::ALL[0], PageTab::Home);
        assert_eq!(PageTab::Home.title(), "Главная");
        let mut titles = std::collections::HashSet::new();
        for tab in PageTab::ALL {
            assert!(titles.insert(tab.title()), "дубликат названия вкладки");
        }
    }

    #[test]
    fn bundled_plus_icon_is_available() {
        let icon_path = IconName::Plus.path();
        let icon = Assets
            .load(icon_path.as_ref())
            .expect("ошибка чтения встроенного значка Plus");

        assert!(icon.is_some(), "встроенный значок Plus отсутствует");
    }
}
