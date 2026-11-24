//! Стартовая страница «Главная»: компактные формы добавления дохода и расхода,
//! краткие описания разделов и ссылки на все страницы.

use super::shell::PageTab;

/// Действие, которое можно выполнить со стартовой страницы.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HomeAction {
    Incomes,
    Expenses,
    Directories,
    Savings,
    Transfers,
    PaymentDocuments,
    Analysis,
    Settings,
}

/// Вкладка, на которую ведёт ссылка со стартовой страницы.
pub(crate) const fn home_action_tab(action: HomeAction) -> PageTab {
    match action {
        HomeAction::Incomes => PageTab::Income,
        HomeAction::Expenses => PageTab::Expenses,
        HomeAction::Directories => PageTab::Directories,
        HomeAction::Savings => PageTab::Savings,
        HomeAction::Transfers => PageTab::Transfers,
        HomeAction::PaymentDocuments => PageTab::PaymentDocuments,
        HomeAction::Analysis => PageTab::Analysis,
        HomeAction::Settings => PageTab::Settings,
    }
}

pub(crate) struct HomeLink {
    pub(crate) title: &'static str,
    pub(crate) description: &'static str,
    pub(crate) action: HomeAction,
}

/// Три основных раздела с пояснением, что в них доступно.
pub(crate) fn described_sections() -> [HomeLink; 3] {
    [
        HomeLink {
            title: "Расходы",
            description: "Учёт списаний: дата, категория, сумма в RUB или USD, место траты и комментарий. Поиск, фильтры и итоги; расход с привязкой к дебетовой карте меняет её остаток.",
            action: HomeAction::Expenses,
        },
        HomeLink {
            title: "Доходы",
            description: "Учёт поступлений: дата, категория, сумма в RUB или USD и комментарий. Поиск, фильтры по периоду, категории, банку и карте; итоги отдельно по валютам.",
            action: HomeAction::Incomes,
        },
        HomeLink {
            title: "Справочники",
            description: "Банки, категории доходов и расходов, места трат, валюты и основная валюта. Используемые записи защищены от удаления, неактивные сохраняются в истории.",
            action: HomeAction::Directories,
        },
    ]
}

/// Ссылки на все страницы приложения, включая три описанных выше раздела.
pub(crate) fn all_page_links() -> [HomeLink; 8] {
    [
        HomeLink {
            title: "Расходы",
            description: "Списания по категориям и местам трат",
            action: HomeAction::Expenses,
        },
        HomeLink {
            title: "Доходы",
            description: "Поступления по категориям и валютам",
            action: HomeAction::Incomes,
        },
        HomeLink {
            title: "Переводы",
            description: "Пополнение, переводы между своими счетами и снятие",
            action: HomeAction::Transfers,
        },
        HomeLink {
            title: "Платёжные карты",
            description: "Дебетовые и кредитные карты, связанные вклады",
            action: HomeAction::PaymentDocuments,
        },
        HomeLink {
            title: "Накопления",
            description: "Вклады и счета с условиями ставок",
            action: HomeAction::Savings,
        },
        HomeLink {
            title: "Анализ",
            description: "Отчёт по периодам, категориям и валютам",
            action: HomeAction::Analysis,
        },
        HomeLink {
            title: "Справочники",
            description: "Банки, категории, места трат и валюты",
            action: HomeAction::Directories,
        },
        HomeLink {
            title: "Настройки",
            description: "Тема, резервное копирование и восстановление",
            action: HomeAction::Settings,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::{all_page_links, described_sections, home_action_tab, HomeAction};
    use crate::gpui::shell::PageTab;

    #[test]
    fn described_sections_cover_three_main_pages() {
        let sections = described_sections();
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].title, "Расходы");
        assert_eq!(sections[1].title, "Доходы");
        assert_eq!(sections[2].title, "Справочники");
        assert_eq!(sections[0].action, HomeAction::Expenses);
        assert_eq!(sections[1].action, HomeAction::Incomes);
        assert_eq!(sections[2].action, HomeAction::Directories);
        assert!(sections.iter().all(|section| {
            let length = section.description.trim().chars().count();
            (1..=200).contains(&length)
        }));
    }

    #[test]
    fn page_links_cover_every_tab_except_home() {
        let links = all_page_links();
        let titles: Vec<_> = links.iter().map(|link| link.title).collect();
        assert_eq!(
            titles,
            [
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
    fn navigation_links_are_unique_and_lead_to_their_tabs() {
        let tabs: Vec<PageTab> = all_page_links()
            .iter()
            .map(|link| home_action_tab(link.action))
            .collect();
        let mut unique = tabs.clone();
        unique.sort_by_key(|tab| *tab as usize);
        unique.dedup();
        assert_eq!(unique.len(), tabs.len());
        assert!(tabs.contains(&PageTab::Transfers));
    }
}
