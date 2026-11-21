//! Стартовая страница «Главная»: краткие описания разделов, быстрые действия
//! и ссылки на все страницы. Финансовых данных и расчётов здесь нет.

use super::shell::PageTab;

/// Действие, которое можно выполнить со стартовой страницы.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HomeAction {
    OpenIncomes,
    OpenExpenses,
    OpenDirectories,
    OpenSavings,
    OpenTransfers,
    OpenPaymentDocuments,
    OpenAnalysis,
    OpenSettings,
    NewIncome,
    NewExpense,
    NewTransfer,
}

/// Вкладка, на которую ведёт действие: быстрые действия открывают форму
/// на странице соответствующего типа операций.
pub(crate) const fn home_action_tab(action: HomeAction) -> PageTab {
    match action {
        HomeAction::OpenIncomes | HomeAction::NewIncome => PageTab::Income,
        HomeAction::OpenExpenses | HomeAction::NewExpense => PageTab::Expenses,
        HomeAction::OpenDirectories => PageTab::Directories,
        HomeAction::OpenSavings => PageTab::Savings,
        HomeAction::OpenTransfers | HomeAction::NewTransfer => PageTab::Transfers,
        HomeAction::OpenPaymentDocuments => PageTab::PaymentDocuments,
        HomeAction::OpenAnalysis => PageTab::Analysis,
        HomeAction::OpenSettings => PageTab::Settings,
    }
}

pub(crate) struct HomeLink {
    pub(crate) title: &'static str,
    pub(crate) description: &'static str,
    pub(crate) action: HomeAction,
}

pub(crate) struct QuickAction {
    pub(crate) label: &'static str,
    pub(crate) action: HomeAction,
}

/// Три основных раздела с пояснением, что в них доступно.
pub(crate) fn described_sections() -> [HomeLink; 3] {
    [
        HomeLink {
            title: "Доходы",
            description: "Учёт поступлений: дата, категория, сумма в RUB или USD и комментарий. Поиск, фильтры по периоду, категории, банку и карте; итоги отдельно по валютам.",
            action: HomeAction::OpenIncomes,
        },
        HomeLink {
            title: "Расходы",
            description: "Учёт списаний: дата, категория, сумма в RUB или USD, место траты и комментарий. Поиск, фильтры и итоги; расход с привязкой к дебетовой карте меняет её остаток.",
            action: HomeAction::OpenExpenses,
        },
        HomeLink {
            title: "Справочники",
            description: "Банки, категории доходов и расходов, места трат, валюты и основная валюта. Используемые записи защищены от удаления, неактивные сохраняются в истории.",
            action: HomeAction::OpenDirectories,
        },
    ]
}

/// Ссылки на все страницы приложения, включая три описанных выше раздела.
pub(crate) fn all_page_links() -> [HomeLink; 8] {
    [
        HomeLink {
            title: "Доходы",
            description: "Поступления по категориям и валютам",
            action: HomeAction::OpenIncomes,
        },
        HomeLink {
            title: "Расходы",
            description: "Списания по категориям и местам трат",
            action: HomeAction::OpenExpenses,
        },
        HomeLink {
            title: "Накопления",
            description: "Вклады и счета с условиями ставок",
            action: HomeAction::OpenSavings,
        },
        HomeLink {
            title: "Переводы",
            description: "Пополнение, переводы между своими счетами и снятие",
            action: HomeAction::OpenTransfers,
        },
        HomeLink {
            title: "Платёжные карты",
            description: "Дебетовые и кредитные карты, связанные вклады",
            action: HomeAction::OpenPaymentDocuments,
        },
        HomeLink {
            title: "Анализ",
            description: "Отчёт по периодам, категориям и валютам",
            action: HomeAction::OpenAnalysis,
        },
        HomeLink {
            title: "Справочники",
            description: "Банки, категории, места трат и валюты",
            action: HomeAction::OpenDirectories,
        },
        HomeLink {
            title: "Настройки",
            description: "Тема, резервное копирование и восстановление",
            action: HomeAction::OpenSettings,
        },
    ]
}

pub(crate) fn quick_actions() -> [QuickAction; 3] {
    [
        QuickAction {
            label: "Добавить доход",
            action: HomeAction::NewIncome,
        },
        QuickAction {
            label: "Добавить расход",
            action: HomeAction::NewExpense,
        },
        QuickAction {
            label: "Добавить перевод",
            action: HomeAction::NewTransfer,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::{all_page_links, described_sections, home_action_tab, quick_actions, HomeAction};
    use crate::gpui::shell::PageTab;

    #[test]
    fn described_sections_cover_three_main_pages() {
        let sections = described_sections();
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].title, "Доходы");
        assert_eq!(sections[1].title, "Расходы");
        assert_eq!(sections[2].title, "Справочники");
        assert_eq!(sections[0].action, HomeAction::OpenIncomes);
        assert_eq!(sections[1].action, HomeAction::OpenExpenses);
        assert_eq!(sections[2].action, HomeAction::OpenDirectories);
        assert!(sections.iter().all(|section| {
            let length = section.description.trim().chars().count();
            (1..=200).contains(&length)
        }));
    }

    #[test]
    fn page_links_cover_every_tab_except_home() {
        let links = all_page_links();
        assert_eq!(links.len(), PageTab::ALL.len() - 1);
        let tabs: Vec<PageTab> = links
            .iter()
            .map(|link| home_action_tab(link.action))
            .collect();
        assert!(PageTab::ALL
            .iter()
            .all(|tab| { *tab == PageTab::Home || tabs.contains(tab) }));
        assert!(links
            .iter()
            .all(|link| !link.title.is_empty() && !link.description.is_empty()));
    }

    #[test]
    fn quick_actions_open_their_pages() {
        let actions = quick_actions();
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].label, "Добавить доход");
        assert_eq!(actions[1].label, "Добавить расход");
        assert_eq!(actions[2].label, "Добавить перевод");
        assert_eq!(home_action_tab(actions[0].action), PageTab::Income);
        assert_eq!(home_action_tab(actions[1].action), PageTab::Expenses);
        assert_eq!(home_action_tab(actions[2].action), PageTab::Transfers);
    }
}
