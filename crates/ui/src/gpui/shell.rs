use std::path::PathBuf;
use std::rc::Rc;

use finance_core::TransactionKind;
use finance_db::Database;

use super::analysis::AnalysisPage;
use super::directories::DirectoriesPage;
use super::home::{all_page_links, described_sections, home_action_tab, HomeAction, HomeLink};
use super::payment_documents::PaymentDocumentsPage;
use super::quick_forms::QuickTransactionForm;
use super::savings::SavingsPage;
use super::selectable::selectable_text;
use super::settings::{SettingsPage, StartFullscreenPreference};
use super::transactions::TransactionsPage;
use super::transfers::TransfersPage;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

/// Вкладки приложения; порядок задаёт [`PageTab::ALL`] и панель вкладок.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PageTab {
    Home,
    Expenses,
    Income,
    Transfers,
    PaymentDocuments,
    Savings,
    Analysis,
    Directories,
    Settings,
}

impl PageTab {
    pub(crate) const ALL: [Self; 9] = [
        Self::Home,
        Self::Expenses,
        Self::Income,
        Self::Transfers,
        Self::PaymentDocuments,
        Self::Savings,
        Self::Analysis,
        Self::Directories,
        Self::Settings,
    ];

    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Home => "Главная",
            Self::Expenses => "Расходы",
            Self::Income => "Доходы",
            Self::Transfers => "Переводы",
            Self::PaymentDocuments => "Платёжные карты",
            Self::Savings => "Накопления",
            Self::Analysis => "Анализ",
            Self::Directories => "Справочники",
            Self::Settings => "Настройки",
        }
    }
}

struct AppShell {
    active: PageTab,
    quick_form: Entity<QuickTransactionForm>,
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
    fn activate(&mut self, tab: PageTab, window: &mut Window, cx: &mut Context<Self>) {
        self.active = tab;
        match tab {
            PageTab::Home => {
                self.quick_form
                    .update(cx, |form, cx| form.reload(window, cx));
            }
            PageTab::Income => self.income.update(cx, |page, cx| page.reload(window, cx)),
            PageTab::Expenses => self.expenses.update(cx, |page, cx| page.reload(window, cx)),
            PageTab::Directories => self
                .directories
                .update(cx, |page, cx| page.reload(window, cx)),
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
                    .flex_1()
                    .min_w_0()
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
                            .child(selectable_text(
                                format!("home-section-description-{}", link.title),
                                link.description,
                            )),
                    )
            })))
            .into_any_element()
    }

    fn render_all_pages_section(&self, links: [HomeLink; 8], cx: &mut Context<Self>) -> AnyElement {
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
                    .child("Функционал"),
            )
            .child(div().flex().flex_wrap().gap_3().children(links.map(|link| {
                let action = link.action;
                div()
                    .w(px(360.0))
                    .flex_none()
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
                            .child(selectable_text(
                                format!("home-page-description-{}", link.title),
                                link.description,
                            )),
                    )
            })))
            .into_any_element()
    }

    fn render_home(&self, cx: &mut Context<Self>) -> AnyElement {
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
        root.child(
            div()
                .w_full()
                .flex()
                .justify_center()
                .child(self.quick_form.clone().into_any_element()),
        )
        .child(self.render_home_section("Основное", described_sections(), cx))
        .child(self.render_all_pages_section(all_page_links(), cx))
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
            let database = Rc::new(database);
            let start_fullscreen = StartFullscreenPreference::load(&database);
            let min_size = size(px(1280.0), px(720.0));
            let options = WindowOptions {
                window_bounds: start_fullscreen
                    .enabled
                    .then(|| WindowBounds::Fullscreen(Bounds::centered(None, min_size, cx))),
                window_min_size: Some(min_size),
                ..WindowOptions::default()
            };
            open_window(options, cx, move |window, cx| {
                let quick_form =
                    cx.new(|cx| QuickTransactionForm::new(database.clone(), window, cx));
                quick_form.update(cx, |form, cx| form.reload(window, cx));
                let income = cx.new(|cx| {
                    TransactionsPage::new(database.clone(), TransactionKind::Income, window, cx)
                });
                let expenses = cx.new(|cx| {
                    TransactionsPage::new(database.clone(), TransactionKind::Expense, window, cx)
                });
                let directories = cx.new(|cx| DirectoriesPage::new(database.clone(), window, cx));
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
                        start_fullscreen,
                        window,
                        cx,
                    )
                });
                cx.new(|_| AppShell {
                    active: PageTab::Home,
                    quick_form,
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
    use super::PageTab;

    #[test]
    fn tabs_follow_the_requested_order_and_have_russian_titles() {
        let titles: Vec<&'static str> = PageTab::ALL.iter().map(|tab| tab.title()).collect();
        assert_eq!(
            titles,
            [
                "Главная",
                "Расходы",
                "Доходы",
                "Переводы",
                "Платёжные карты",
                "Накопления",
                "Анализ",
                "Справочники",
                "Настройки",
            ]
        );
        assert_eq!(PageTab::ALL.len(), 9);
        assert_eq!(PageTab::ALL[0], PageTab::Home);
        assert_eq!(PageTab::Home.title(), "Главная");
        let mut unique = std::collections::HashSet::new();
        for title in titles {
            assert!(unique.insert(title), "дубликат названия вкладки");
        }
    }
}
