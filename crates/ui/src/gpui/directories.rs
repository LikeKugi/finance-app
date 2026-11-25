//! Страница «Справочники» на `gpui-kit`: категории доходов и расходов, банки
//! и валюты. Правила защиты используемых записей остаются в `finance-db`.

use std::collections::HashSet;
use std::rc::Rc;
use std::str::FromStr;

use chrono::{Local, NaiveDate};
use finance_core::{Bank, Category, CurrencyDefinition, ExpensePlace, TransactionKind};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{AlertDialog, Dialog};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::table::{
    Column, ColumnFixed, DataTable, TableDelegate, TableEvent, TableState,
};
use gpui_kit::component::tooltip::Tooltip as ComponentTooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::StatefulInteractiveElement;
use gpui_kit::*;
use rust_decimal::Decimal;

use super::helpers::DATE_FORMAT;
use super::prices::PricesSection;
use super::selectable::{selectable_message, selectable_table_cell, selectable_text};
use super::support::category_tree_labels;
use super::support::category_tree_rows;
use super::support::collect_category_descendants;
use super::support::currency_rate_label;
use super::transactions::comment_preview;

const NO_PARENT_LABEL: &str = "Нет";
const NO_PARENT_TOOLTIP: &str = "Без родительской категории";
const VISIBLE_TABLE_ROWS: usize = 10;
const TABLE_ROW_HEIGHT: f32 = 32.0;
/// Высота таблицы: 10 строк данных плюс строка заголовка.
const TABLE_HEIGHT: f32 = TABLE_ROW_HEIGHT * (VISIBLE_TABLE_ROWS as f32 + 1.0);
const ACTIONS_WIDTH: f32 = 160.0;
/// Ширина колонки «Название» в таблице банков. Используется как ориентир для
/// ширины основной части данных таблицы валют.
const BANK_NAME_COLUMN_WIDTH: f32 = 480.0;
/// Ширины колонок таблицы валют подобраны так, чтобы она помещалась в правую
/// половину страницы при минимальном разрешении 1280×720.
const CURRENCY_COLUMN_WIDTH: f32 = 80.0;
const CURRENCY_RATE_COLUMN_WIDTH: f32 = 160.0;
/// «Валюта», «Курс» и «Основная» вместе занимают столько же, сколько колонка
/// «Название» в таблице банков. Этого достаточно для полной подписи кнопки
/// «Назначить основной», а ширина всей таблицы вместе с «Действиями»
/// совпадает с шириной таблицы банков.
const CURRENCY_PRIMARY_COLUMN_WIDTH: f32 =
    BANK_NAME_COLUMN_WIDTH - CURRENCY_COLUMN_WIDTH - CURRENCY_RATE_COLUMN_WIDTH;
const CURRENCY_ACTIONS_WIDTH: f32 = 160.0;
const MAKE_PRIMARY_BUTTON_LABEL: &str = "Назначить основной";
const EDIT_CURRENCY_BUTTON_LABEL: &str = "Изменить";
/// Категории расходов выбраны при первом открытии страницы.
const DEFAULT_CATEGORY_KIND: TransactionKind = TransactionKind::Expense;
/// Порядок переключателя видов категорий на странице «Справочники».
const CATEGORY_KINDS: [TransactionKind; 2] = [DEFAULT_CATEGORY_KIND, TransactionKind::Income];

fn category_kind_toggle_id(kind: TransactionKind) -> &'static str {
    match kind {
        TransactionKind::Income => "kind-income",
        TransactionKind::Expense => "kind-expense",
    }
}

fn category_kind_toggle_label(kind: TransactionKind) -> &'static str {
    match kind {
        TransactionKind::Income => "Категории доходов",
        TransactionKind::Expense => "Категории расходов",
    }
}

type ParentChoice = Entity<SelectState<Vec<ParentCategoryItem>>>;

#[derive(Clone)]
struct ParentCategoryItem {
    id: usize,
    label: String,
}

impl SelectItem for ParentCategoryItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.label
    }

    fn render(&self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tooltip = if self.label == NO_PARENT_LABEL {
            NO_PARENT_TOOLTIP.to_owned()
        } else {
            self.label.clone()
        };
        div()
            .id(("parent-category-option", self.id))
            .w_full()
            .tooltip(move |window, cx| ComponentTooltip::new(tooltip.clone()).build(window, cx))
            .child(self.label.clone())
    }
}

#[derive(Clone)]
struct CategoryRow {
    category: Category,
    depth: usize,
}

#[derive(Clone)]
struct CurrencyRow {
    currency: CurrencyDefinition,
    rate_label: String,
    used: bool,
}

struct CategoriesTable {
    page: WeakEntity<DirectoriesPage>,
    rows: Vec<CategoryRow>,
}

struct BanksTable {
    page: WeakEntity<DirectoriesPage>,
    rows: Vec<Bank>,
}

struct ExpensePlacesTable {
    page: WeakEntity<DirectoriesPage>,
    rows: Vec<ExpensePlace>,
}

struct CurrenciesTable {
    page: WeakEntity<DirectoriesPage>,
    rows: Vec<CurrencyRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RightTab {
    Banks,
    ExpensePlaces,
    Currencies,
}

impl RightTab {
    /// Порядок переключателя справочников в правой половине страницы.
    const ALL: [Self; 3] = [Self::Banks, Self::ExpensePlaces, Self::Currencies];

    const fn toggle_id(self) -> &'static str {
        match self {
            Self::Banks => "tab-banks",
            Self::ExpensePlaces => "tab-expense-places",
            Self::Currencies => "tab-currencies",
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Banks => "Банки",
            Self::ExpensePlaces => "Места покупок",
            Self::Currencies => "Валюты",
        }
    }
}

struct CategoryForm {
    id: Option<i64>,
    name: Entity<InputState>,
    parent_ids: Vec<Option<i64>>,
    parent_labels: Vec<String>,
    parent_id: Option<i64>,
    parent_select: ParentChoice,
    is_active: bool,
}

struct BankForm {
    id: Option<i64>,
    name: Entity<InputState>,
    comment: Entity<InputState>,
}

struct ExpensePlaceForm {
    id: Option<i64>,
    name: Entity<InputState>,
    comment: Entity<TextareaState>,
}

struct CurrencyForm {
    /// `Some(code)` — правка курса существующей валюты, `None` — новая валюта.
    rate_for: Option<String>,
    code: Entity<InputState>,
    name: Entity<InputState>,
    rate: Entity<InputState>,
    rate_date: Entity<InputState>,
}

#[derive(Clone, Copy)]
enum FormKind {
    Category,
    Bank,
    Currency,
    ExpensePlace,
}

enum Delete {
    Category(i64),
    Bank(i64),
    Currency(String),
    ExpensePlace(i64),
}

pub(crate) struct DirectoriesPage {
    database: Rc<Database>,
    kind: TransactionKind,
    right_tab: RightTab,
    categories: Vec<Category>,
    banks: Vec<Bank>,
    expense_places: Vec<ExpensePlace>,
    currencies: Vec<CurrencyDefinition>,
    used_currencies: HashSet<String>,
    message: Option<(String, bool)>,
    category_form: Option<CategoryForm>,
    bank_form: Option<BankForm>,
    expense_place_form: Option<ExpensePlaceForm>,
    currency_form: Option<CurrencyForm>,
    categories_table: Entity<TableState<CategoriesTable>>,
    banks_table: Entity<TableState<BanksTable>>,
    expense_places_table: Entity<TableState<ExpensePlacesTable>>,
    currencies_table: Entity<TableState<CurrenciesTable>>,
    prices: Entity<PricesSection>,
    _subscriptions: Vec<Subscription>,
}

fn input(
    placeholder: &str,
    value: &str,
    window: &mut Window,
    cx: &mut Context<DirectoriesPage>,
) -> Entity<InputState> {
    let placeholder = placeholder.to_owned();
    let value = value.to_owned();
    cx.new(|cx| {
        let mut state = InputState::new(window, cx).placeholder(placeholder);
        if !value.is_empty() {
            state.set_value(value, window, cx);
        }
        state
    })
}

/// Варианты родителя в том же порядке дерева, что и фильтр категорий на
/// страницах операций: сначала корневые категории, затем их потомки с полным
/// путём. Редактируемая категория и её потомки исключаются, чтобы нельзя было
/// выбрать родителя, образующего цикл.
fn parent_category_options(
    categories: &[Category],
    own_id: Option<i64>,
) -> Vec<(Option<i64>, String)> {
    let excluded: HashSet<i64> = own_id
        .map(|id| {
            collect_category_descendants(categories, id)
                .into_iter()
                .collect()
        })
        .unwrap_or_default();
    let available: Vec<Category> = categories
        .iter()
        .filter(|category| !excluded.contains(&category.id))
        .cloned()
        .collect();

    let mut options = vec![(None, NO_PARENT_LABEL.to_owned())];
    options.extend(
        category_tree_labels(&available)
            .into_iter()
            .map(|(id, label)| (Some(id), label)),
    );
    options
}

fn directory_column(key: &'static str, name: &'static str, width: f32, actions: bool) -> Column {
    let mut column = Column::new(key, name)
        .width(px(width))
        .min_width(px(width))
        .max_width(px(width))
        .resizable(false)
        .movable(false)
        .selectable(!actions);
    if actions {
        column = column.fixed(ColumnFixed::Right);
    }
    column
}

fn directory_cell(content: AnyElement, col_ix: usize, cx: &App) -> AnyElement {
    let border = cx.theme().border;
    div()
        .w_full()
        .h_full()
        .flex()
        .items_center()
        .overflow_hidden()
        .when(col_ix > 0, |cell| cell.border_l_1().border_color(border))
        .child(content)
        .into_any_element()
}

/// В таблице показываем только код базовых валют: полное название дублирует
/// код и занимает место в узкой колонке. Остальные валюты подписаны подробнее.
fn currency_cell_label(currency: &CurrencyDefinition) -> String {
    match currency.code.as_str() {
        "RUB" | "USD" => currency.code.clone(),
        _ => format!("{} {}", currency.code, currency.name),
    }
}

/// Предпочтительные ширины колонок таблицы валют. Контейнер таблицы занимает
/// весь правый блок страницы, как и таблицы банков и мест покупок.
fn currency_column_widths() -> [f32; 4] {
    [
        CURRENCY_COLUMN_WIDTH,
        CURRENCY_RATE_COLUMN_WIDTH,
        CURRENCY_PRIMARY_COLUMN_WIDTH,
        CURRENCY_ACTIONS_WIDTH,
    ]
}

impl DirectoriesPage {
    pub(crate) fn new(database: Rc<Database>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prices = cx.new(|cx| PricesSection::new(database.clone(), window, cx));
        let page_ref = cx.entity().downgrade();
        let categories_table = cx.new(|cx| {
            TableState::new(
                CategoriesTable {
                    page: page_ref.clone(),
                    rows: Vec::new(),
                },
                window,
                cx,
            )
            .sortable(false)
            .col_movable(false)
            .col_resizable(false)
            .row_selectable(true)
            .col_selectable(false)
        });
        let banks_table = cx.new(|cx| {
            TableState::new(
                BanksTable {
                    page: page_ref.clone(),
                    rows: Vec::new(),
                },
                window,
                cx,
            )
            .sortable(false)
            .col_movable(false)
            .col_resizable(false)
            .row_selectable(true)
            .col_selectable(false)
        });
        let expense_places_table = cx.new(|cx| {
            TableState::new(
                ExpensePlacesTable {
                    page: page_ref.clone(),
                    rows: Vec::new(),
                },
                window,
                cx,
            )
            .sortable(false)
            .col_movable(false)
            .col_resizable(false)
            .row_selectable(true)
            .col_selectable(false)
        });
        let currencies_table = cx.new(|cx| {
            TableState::new(
                CurrenciesTable {
                    page: page_ref.clone(),
                    rows: Vec::new(),
                },
                window,
                cx,
            )
            .sortable(false)
            .col_movable(false)
            .col_resizable(false)
            .row_selectable(true)
            .col_selectable(false)
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &categories_table,
                window,
                |page: &mut Self, _, event: &TableEvent, window, cx| {
                    let TableEvent::DoubleClickedRow(row_ix) = event else {
                        return;
                    };
                    let Some(category) = page
                        .categories_table
                        .read(cx)
                        .delegate()
                        .rows
                        .get(*row_ix)
                        .map(|row| row.category.clone())
                    else {
                        return;
                    };
                    page.open_category_form(Some(&category), window, cx);
                },
            ),
            cx.subscribe_in(
                &banks_table,
                window,
                |page: &mut Self, _, event: &TableEvent, window, cx| {
                    let TableEvent::DoubleClickedRow(row_ix) = event else {
                        return;
                    };
                    let Some(bank) = page
                        .banks_table
                        .read(cx)
                        .delegate()
                        .rows
                        .get(*row_ix)
                        .cloned()
                    else {
                        return;
                    };
                    page.open_bank_form(Some(&bank), window, cx);
                },
            ),
            cx.subscribe_in(
                &expense_places_table,
                window,
                |page: &mut Self, _, event: &TableEvent, window, cx| {
                    let TableEvent::DoubleClickedRow(row_ix) = event else {
                        return;
                    };
                    let Some(place) = page
                        .expense_places_table
                        .read(cx)
                        .delegate()
                        .rows
                        .get(*row_ix)
                        .cloned()
                    else {
                        return;
                    };
                    page.open_expense_place_form(Some(&place), window, cx);
                },
            ),
            cx.subscribe_in(
                &currencies_table,
                window,
                |page: &mut Self, _, event: &TableEvent, window, cx| {
                    let TableEvent::DoubleClickedRow(row_ix) = event else {
                        return;
                    };
                    let Some(currency) = page
                        .currencies_table
                        .read(cx)
                        .delegate()
                        .rows
                        .get(*row_ix)
                        .map(|row| row.currency.clone())
                    else {
                        return;
                    };
                    if !currency.is_primary {
                        page.open_currency_form(Some(&currency), window, cx);
                    }
                },
            ),
        ];
        let mut page = Self {
            database,
            prices,
            kind: DEFAULT_CATEGORY_KIND,
            right_tab: RightTab::Banks,
            categories: Vec::new(),
            banks: Vec::new(),
            expense_places: Vec::new(),
            currencies: Vec::new(),
            used_currencies: HashSet::new(),
            message: None,
            category_form: None,
            bank_form: None,
            expense_place_form: None,
            currency_form: None,
            categories_table,
            banks_table,
            expense_places_table,
            currencies_table,
            _subscriptions: subscriptions,
        };
        page.reload(window, cx);
        page
    }

    fn error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.message = Some((message.into(), true));
        cx.notify();
    }

    /// Перечитывает все справочники; ошибки показываются, данные не подменяются пустыми.
    pub(crate) fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let db = &self.database;
        let categories = finance_db::list_categories(db, self.kind, true);
        let banks = finance_db::list_banks(db);
        let expense_places = finance_db::list_expense_places(db);
        let currencies = finance_db::list_currencies(db);
        let used = finance_db::list_used_currency_codes(db);
        let mut failure = None;
        match categories {
            Ok(v) => self.categories = v,
            Err(e) => failure = Some(e.to_string()),
        }
        match banks {
            Ok(v) => self.banks = v,
            Err(e) => failure = Some(e.to_string()),
        }
        match expense_places {
            Ok(v) => self.expense_places = v,
            Err(e) => failure = Some(e.to_string()),
        }
        match currencies {
            Ok(v) => self.currencies = v,
            Err(e) => {
                self.currencies.clear();
                failure = Some(e.to_string());
            }
        }
        match used {
            Ok(v) => self.used_currencies = v.into_iter().collect(),
            Err(e) => failure = Some(e.to_string()),
        }
        match failure {
            Some(message) => self.error(message, cx),
            None => {
                self.message = None;
                cx.notify();
            }
        }
        self.sync_tables(cx);
        self.prices
            .update(cx, |prices, cx| prices.reload_places(window, cx));
    }

    fn sync_tables(&mut self, cx: &mut Context<Self>) {
        let category_rows = category_tree_rows(&self.categories)
            .into_iter()
            .map(|(index, depth)| CategoryRow {
                category: self.categories[index].clone(),
                depth,
            })
            .collect();
        let primary_code = self
            .currencies
            .iter()
            .find(|currency| currency.is_primary)
            .map_or("—".to_owned(), |currency| currency.code.clone());
        let currency_rows = self
            .currencies
            .iter()
            .map(|currency| CurrencyRow {
                rate_label: currency_rate_label(currency, &primary_code),
                used: self.used_currencies.contains(&currency.code),
                currency: currency.clone(),
            })
            .collect();

        self.categories_table.update(cx, |table, cx| {
            table.delegate_mut().rows = category_rows;
            cx.notify();
        });
        self.banks_table.update(cx, |table, cx| {
            table.delegate_mut().rows = self.banks.clone();
            cx.notify();
        });
        self.expense_places_table.update(cx, |table, cx| {
            table.delegate_mut().rows = self.expense_places.clone();
            cx.notify();
        });
        self.currencies_table.update(cx, |table, cx| {
            table.delegate_mut().rows = currency_rows;
            cx.notify();
        });
    }

    fn set_kind(&mut self, kind: TransactionKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.kind != kind {
            self.kind = kind;
            self.category_form = None;
            self.reload(window, cx);
        }
    }

    fn set_right_tab(&mut self, tab: RightTab, cx: &mut Context<Self>) {
        if self.right_tab != tab {
            self.right_tab = tab;
            self.bank_form = None;
            self.expense_place_form = None;
            self.currency_form = None;
            cx.notify();
        }
    }

    // ---- категории ----

    fn open_category_form(
        &mut self,
        existing: Option<&Category>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = input(
            "Название",
            existing.map_or("", |c| c.name.as_str()),
            window,
            cx,
        );
        let own_id = existing.map(|c| c.id);
        let options = parent_category_options(&self.categories, own_id);
        let parent_ids: Vec<Option<i64>> = options.iter().map(|(id, _)| *id).collect();
        let parent_labels: Vec<String> = options.into_iter().map(|(_, label)| label).collect();
        let items: Vec<ParentCategoryItem> = parent_labels
            .iter()
            .cloned()
            .enumerate()
            .map(|(id, label)| ParentCategoryItem { id, label })
            .collect();
        let parent_id = existing.and_then(|c| c.parent_id);
        let selected = parent_ids.iter().position(|p| *p == parent_id).unwrap_or(0);
        let parent_select = cx.new(|cx| {
            SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx)
        });
        let sub = cx.subscribe_in(
            &parent_select,
            window,
            |this: &mut Self, state, event: &SelectEvent<Vec<ParentCategoryItem>>, _, cx| {
                let SelectEvent::Confirm(Some(_)) = event else {
                    return;
                };
                let row = state.read(cx).selected_index(cx).map(|ix| ix.row);
                if let (Some(form), Some(row)) = (this.category_form.as_mut(), row) {
                    form.parent_id = form.parent_ids.get(row).copied().flatten();
                }
                cx.notify();
            },
        );
        self._subscriptions.push(sub);
        self.category_form = Some(CategoryForm {
            id: own_id,
            name,
            parent_ids,
            parent_labels,
            parent_id,
            parent_select,
            is_active: existing.is_none_or(|c| c.is_active),
        });
        self.message = None;
        cx.notify();
        self.open_form_dialog(FormKind::Category, window, cx);
    }

    fn save_category(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.category_form.as_ref() else {
            return;
        };
        let name = form.name.read(cx).value().to_string();
        let result = finance_db::save_category(
            &self.database,
            self.kind,
            form.id,
            &name,
            form.parent_id,
            form.is_active,
        );
        match result {
            Ok(_) => {
                self.category_form = None;
                window.close_dialog(cx);
                self.reload(window, cx);
            }
            Err(error) => self.error(error.to_string(), cx),
        }
    }

    // ---- банки ----

    fn open_bank_form(
        &mut self,
        existing: Option<&Bank>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.bank_form = Some(BankForm {
            id: existing.map(|b| b.id),
            name: input(
                "Название",
                existing.map_or("", |b| b.name.as_str()),
                window,
                cx,
            ),
            comment: input(
                "Комментарий",
                existing.map_or("", |b| b.comment.as_str()),
                window,
                cx,
            ),
        });
        self.message = None;
        cx.notify();
        self.open_form_dialog(FormKind::Bank, window, cx);
    }

    fn save_bank(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.bank_form.as_ref() else {
            return;
        };
        let name = form.name.read(cx).value().to_string();
        let comment = form.comment.read(cx).value().to_string();
        match finance_db::save_bank(&self.database, form.id, &name, &comment) {
            Ok(_) => {
                self.bank_form = None;
                window.close_dialog(cx);
                self.reload(window, cx);
            }
            Err(error) => self.error(error.to_string(), cx),
        }
    }

    // ---- места покупок ----

    fn open_expense_place_form(
        &mut self,
        existing: Option<&ExpensePlace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let comment = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx).rows(4);
            if let Some(comment) = existing.and_then(|place| place.comment.as_ref()) {
                state.set_value(comment.clone(), window, cx);
            }
            state
        });
        self.expense_place_form = Some(ExpensePlaceForm {
            id: existing.map(|place| place.id),
            name: input(
                "Название",
                existing.map_or("", |place| place.name.as_str()),
                window,
                cx,
            ),
            comment,
        });
        self.message = None;
        cx.notify();
        self.open_form_dialog(FormKind::ExpensePlace, window, cx);
    }

    fn save_expense_place(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.expense_place_form.as_ref() else {
            return;
        };
        let name = form.name.read(cx).value().to_string();
        let comment = form.comment.read(cx).value().to_string();
        let comment = (!comment.is_empty()).then_some(comment.as_str());
        match finance_db::save_expense_place(&self.database, form.id, &name, comment) {
            Ok(_) => {
                self.expense_place_form = None;
                window.close_dialog(cx);
                self.reload(window, cx);
            }
            Err(error) => self.error(error.to_string(), cx),
        }
    }

    // ---- валюты ----

    fn open_currency_form(
        &mut self,
        rate_for: Option<&CurrencyDefinition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let today = Local::now().date_naive();
        self.currency_form = Some(CurrencyForm {
            rate_for: rate_for.map(|c| c.code.clone()),
            code: input("Код: 3 заглавные латинские буквы", "", window, cx),
            name: input("Название", "", window, cx),
            rate: input(
                "За 1 единицу валюты",
                &rate_for
                    .and_then(|c| c.rate_to_primary)
                    .map_or_else(String::new, |r| r.to_string()),
                window,
                cx,
            ),
            rate_date: input(
                "ГГГГ-ММ-ДД",
                &rate_for
                    .and_then(|c| c.rate_date)
                    .unwrap_or(today)
                    .format(DATE_FORMAT)
                    .to_string(),
                window,
                cx,
            ),
        });
        self.message = None;
        cx.notify();
        self.open_form_dialog(FormKind::Currency, window, cx);
    }

    fn save_currency(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.currency_form.as_ref() else {
            return;
        };
        let result = if let Some(code) = &form.rate_for {
            let rate_text = form.rate.read(cx).value().trim().replace(',', ".");
            let Ok(rate) = Decimal::from_str(&rate_text) else {
                self.error("Введите корректный положительный курс", cx);
                return;
            };
            let date_text = form.rate_date.read(cx).value().to_string();
            let Ok(date) = NaiveDate::parse_from_str(date_text.trim(), DATE_FORMAT) else {
                self.error("Введите дату курса в формате ГГГГ-ММ-ДД", cx);
                return;
            };
            finance_db::set_currency_exchange_rate(&self.database, code, rate, date)
        } else {
            let code = form.code.read(cx).value().trim().to_ascii_uppercase();
            let name = form.name.read(cx).value().to_string();
            finance_db::add_currency(&self.database, &code, &name)
        };
        match result {
            Ok(()) => {
                self.currency_form = None;
                window.close_dialog(cx);
                self.reload(window, cx);
            }
            Err(error) => self.error(error.to_string(), cx),
        }
    }

    fn make_primary(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        match finance_db::set_primary_currency(&self.database, code) {
            Ok(()) => self.reload(window, cx),
            Err(error) => self.error(error.to_string(), cx),
        }
    }

    // ---- удаление ----

    fn confirm_delete(
        &mut self,
        target: Delete,
        subject: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity();
        let description = format!("Удалить {subject}?");
        window.open_alert_dialog(cx, move |alert: AlertDialog, _, _| {
            let this = this.clone();
            let target = match &target {
                Delete::Category(id) => Delete::Category(*id),
                Delete::Bank(id) => Delete::Bank(*id),
                Delete::Currency(code) => Delete::Currency(code.clone()),
                Delete::ExpensePlace(id) => Delete::ExpensePlace(*id),
            };
            alert
                .title("Подтверждение удаления")
                .description(description.clone())
                .ok_text("Удалить")
                .cancel_text("Отмена")
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    this.update(cx, |page, cx| {
                        let result = match &target {
                            Delete::Category(id) => {
                                finance_db::delete_category(&page.database, page.kind, *id)
                            }
                            Delete::Bank(id) => finance_db::delete_bank(&page.database, *id),
                            Delete::Currency(code) => {
                                finance_db::delete_currency(&page.database, code)
                            }
                            Delete::ExpensePlace(id) => {
                                finance_db::delete_expense_place(&page.database, *id)
                            }
                        };
                        match result {
                            Ok(()) => page.reload(window, cx),
                            Err(error) => page.error(error.to_string(), cx),
                        }
                    });
                    true
                })
        });
    }

    // ---- формы в диалогах ----

    fn open_form_dialog(&self, kind: FormKind, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity();
        window.open_dialog(cx, move |dialog: Dialog, _, cx| {
            let (title, body) = this.update(cx, |page, cx| page.form_dialog_content(kind, cx));
            dialog
                .title(title)
                .centered()
                .w(px(560.0))
                .close_button(false)
                .overlay_closable(false)
                .keyboard(false)
                .child(body)
        });
    }

    fn form_dialog_content(
        &mut self,
        kind: FormKind,
        cx: &mut Context<Self>,
    ) -> (String, AnyElement) {
        let danger = cx.theme().danger;
        let (title, fields, id, save_label) = match kind {
            FormKind::Category => {
                let Some(form) = self.category_form.as_ref() else {
                    return (String::new(), div().into_any_element());
                };
                let title = match (form.id.is_some(), self.kind) {
                    (true, TransactionKind::Income) => "Изменить категорию доходов",
                    (true, TransactionKind::Expense) => "Изменить категорию расходов",
                    (false, TransactionKind::Income) => "Новая категория доходов",
                    (false, TransactionKind::Expense) => "Новая категория расходов",
                };
                let parent_tooltip = form
                    .parent_select
                    .read(cx)
                    .selected_index(cx)
                    .and_then(|index| form.parent_labels.get(index.row))
                    .map(|label| {
                        if label == NO_PARENT_LABEL {
                            NO_PARENT_TOOLTIP.to_owned()
                        } else {
                            label.clone()
                        }
                    });
                let fields = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child("Название")
                    .child(Input::new(&form.name))
                    .child("Родительская категория")
                    .child(
                        div()
                            .id("parent-category-select")
                            .child(Select::new(&form.parent_select))
                            .when_some(parent_tooltip, |el, tooltip| {
                                el.tooltip(move |window, cx| {
                                    ComponentTooltip::new(tooltip.clone()).build(window, cx)
                                })
                            }),
                    )
                    .child(
                        Checkbox::new("category-active")
                            .label("Активна")
                            .checked(form.is_active)
                            .on_click(cx.listener(|page, checked: &bool, _, cx| {
                                if let Some(form) = page.category_form.as_mut() {
                                    form.is_active = *checked;
                                }
                                cx.notify();
                            })),
                    );
                (title.to_owned(), fields, "category", "Сохранить категорию")
            }
            FormKind::Bank => {
                let Some(form) = self.bank_form.as_ref() else {
                    return (String::new(), div().into_any_element());
                };
                let title = if form.id.is_some() {
                    "Изменить банк"
                } else {
                    "Новый банк"
                };
                let fields = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child("Название")
                    .child(Input::new(&form.name))
                    .child("Комментарий")
                    .child(Input::new(&form.comment));
                (title.to_owned(), fields, "bank", "Сохранить банк")
            }
            FormKind::ExpensePlace => {
                let Some(form) = self.expense_place_form.as_ref() else {
                    return (String::new(), div().into_any_element());
                };
                let title = if form.id.is_some() {
                    "Изменить место покупок"
                } else {
                    "Новое место покупок"
                };
                let fields = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child("Название")
                    .child(Input::new(&form.name))
                    .child("Комментарий")
                    .child(Textarea::new(&form.comment));
                (title.to_owned(), fields, "expense-place", "Сохранить место")
            }
            FormKind::Currency => {
                let Some(form) = self.currency_form.as_ref() else {
                    return (String::new(), div().into_any_element());
                };
                let primary_code = self
                    .currencies
                    .iter()
                    .find(|c| c.is_primary)
                    .map_or("—".to_owned(), |c| c.code.clone());
                let title = match &form.rate_for {
                    Some(code) => format!("Курс {code} к {primary_code}"),
                    None => "Новая валюта".to_owned(),
                };
                let fields = div().flex().flex_col().gap_2();
                let fields = if form.rate_for.is_some() {
                    fields
                        .child("За 1 единицу валюты")
                        .child(Input::new(&form.rate))
                        .child("Дата курса (ГГГГ-ММ-ДД)")
                        .child(Input::new(&form.rate_date))
                } else {
                    fields
                        .child("Код (3 заглавные латинские буквы)")
                        .child(Input::new(&form.code))
                        .child("Название")
                        .child(Input::new(&form.name))
                };
                (title, fields, "currency", "Сохранить")
            }
        };
        let error = self.message.clone();
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(fields)
            .when_some(error, |body, (message, _)| {
                body.child(selectable_message("directory-form-error", message, danger))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("{id}-cancel")))
                            .label("Отмена")
                            .on_click(cx.listener(move |page, _, window, cx| {
                                match kind {
                                    FormKind::Category => page.category_form = None,
                                    FormKind::Bank => page.bank_form = None,
                                    FormKind::ExpensePlace => page.expense_place_form = None,
                                    FormKind::Currency => page.currency_form = None,
                                }
                                page.message = None;
                                window.close_dialog(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{id}-save")))
                            .primary()
                            .label(save_label)
                            .on_click(cx.listener(move |page, _, window, cx| match kind {
                                FormKind::Category => page.save_category(window, cx),
                                FormKind::Bank => page.save_bank(window, cx),
                                FormKind::ExpensePlace => page.save_expense_place(window, cx),
                                FormKind::Currency => page.save_currency(window, cx),
                            })),
                    ),
            );
        (title, body.into_any_element())
    }

    // ---- отрисовка ----

    fn render_categories(&mut self, cx: &mut Context<Self>) -> Div {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::BOLD)
                            .child("Категории"),
                    )
                    .child(
                        Button::new("category-add")
                            .primary()
                            .small()
                            .label("Добавить категорию")
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|page, _, window, cx| {
                                page.open_category_form(None, window, cx)
                            })),
                    ),
            )
            .child(
                div().h(px(TABLE_HEIGHT)).child(
                    DataTable::new(&self.categories_table)
                        .stripe(true)
                        .bordered(true)
                        .accessibility_label("Категории"),
                ),
            )
    }

    fn render_banks(&mut self, _: &mut Context<Self>) -> Div {
        div().h(px(TABLE_HEIGHT)).w_full().child(
            DataTable::new(&self.banks_table)
                .stripe(true)
                .bordered(true)
                .accessibility_label("Банки"),
        )
    }

    fn render_expense_places(&mut self, _: &mut Context<Self>) -> Div {
        div().h(px(TABLE_HEIGHT)).w_full().child(
            DataTable::new(&self.expense_places_table)
                .stripe(true)
                .bordered(true)
                .accessibility_label("Места покупок"),
        )
    }

    fn render_currencies(&mut self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap_2()
            .child(
                div()
                    .h(px(TABLE_HEIGHT))
                    .w_full()
                    .child(
                        DataTable::new(&self.currencies_table)
                            .stripe(true)
                            .bordered(true)
                            .accessibility_label("Валюты"),
                    ),
            )
            .child(selectable_message(
                "currencies-rate-note",
                "Курс означает количество единиц основной валюты за 1 единицу выбранной валюты.",
                muted,
            ))
            .child(selectable_message(
                "currencies-limitation-note",
                "Дополнительные валюты пока нельзя выбирать в формах доходов и расходов (поддерживаются RUB и USD).",
                muted,
            ))
    }
}

impl TableDelegate for CategoriesTable {
    fn columns_count(&self, _: &App) -> usize {
        2
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => directory_column("name", "Название", 480.0, false),
            _ => directory_column("actions", "Действия", ACTIONS_WIDTH, true),
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(selectable_text("categories-empty", "Категории не найдены."))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = self.rows[row_ix].clone();
        let content = if col_ix == 0 {
            let prefix = if row.depth == 0 {
                String::new()
            } else {
                format!("{}> ", "  ".repeat(row.depth))
            };
            let suffix = if row.category.is_active {
                ""
            } else {
                " (неактивна)"
            };
            div()
                .w_full()
                .truncate()
                .child(selectable_table_cell(
                    "categories",
                    row.category.id,
                    col_ix,
                    format!("{prefix}{}{suffix}", row.category.name),
                ))
                .into_any_element()
        } else {
            let id = row.category.id;
            let edit_page = self.page.clone();
            let edit = row.category.clone();
            let delete_page = self.page.clone();
            let subject = format!("категорию «{}»", row.category.name);
            div()
                .flex()
                .items_center()
                .justify_center()
                .gap_1()
                .child(
                    Button::new(("category-edit", row_ix))
                        .xsmall()
                        .label("Изменить")
                        .on_click(move |_, window, cx| {
                            let edit = edit.clone();
                            let _ = edit_page.update(cx, |page, cx| {
                                page.open_category_form(Some(&edit), window, cx)
                            });
                        }),
                )
                .child(
                    Button::new(("category-delete", row_ix))
                        .danger()
                        .xsmall()
                        .label("Удалить")
                        .on_click(move |_, window, cx| {
                            let subject = subject.clone();
                            let _ = delete_page.update(cx, |page, cx| {
                                page.confirm_delete(Delete::Category(id), subject, window, cx)
                            });
                        }),
                )
                .into_any_element()
        };
        directory_cell(content, col_ix, cx)
    }

    fn cell_label(&self, row_ix: usize, col_ix: usize, _: &App) -> Option<SharedString> {
        let row = &self.rows[row_ix];
        Some(
            if col_ix == 0 {
                row.category.name.clone()
            } else {
                "Действия".to_owned()
            }
            .into(),
        )
    }
}

impl TableDelegate for BanksTable {
    fn columns_count(&self, _: &App) -> usize {
        2
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => directory_column("name", "Название", BANK_NAME_COLUMN_WIDTH, false),
            _ => directory_column("actions", "Действия", ACTIONS_WIDTH, true),
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(selectable_text("banks-empty", "Банки не найдены."))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let bank = self.rows[row_ix].clone();
        let content = if col_ix == 0 {
            div()
                .w_full()
                .truncate()
                .child(selectable_table_cell(
                    "banks",
                    bank.id,
                    col_ix,
                    bank.name.clone(),
                ))
                .into_any_element()
        } else {
            let id = bank.id;
            let edit_page = self.page.clone();
            let edit = bank.clone();
            let delete_page = self.page.clone();
            let subject = format!("банк «{}»", bank.name);
            div()
                .flex()
                .items_center()
                .justify_center()
                .gap_1()
                .child(
                    Button::new(("bank-edit", row_ix))
                        .xsmall()
                        .label("Изменить")
                        .on_click(move |_, window, cx| {
                            let edit = edit.clone();
                            let _ = edit_page.update(cx, |page, cx| {
                                page.open_bank_form(Some(&edit), window, cx)
                            });
                        }),
                )
                .child(
                    Button::new(("bank-delete", row_ix))
                        .danger()
                        .xsmall()
                        .label("Удалить")
                        .on_click(move |_, window, cx| {
                            let subject = subject.clone();
                            let _ = delete_page.update(cx, |page, cx| {
                                page.confirm_delete(Delete::Bank(id), subject, window, cx)
                            });
                        }),
                )
                .into_any_element()
        };
        directory_cell(content, col_ix, cx)
    }

    fn cell_label(&self, row_ix: usize, col_ix: usize, _: &App) -> Option<SharedString> {
        let bank = &self.rows[row_ix];
        Some(
            if col_ix == 0 {
                bank.name.clone()
            } else {
                "Действия".to_owned()
            }
            .into(),
        )
    }
}

impl TableDelegate for ExpensePlacesTable {
    fn columns_count(&self, _: &App) -> usize {
        3
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => directory_column("name", "Название", 220.0, false),
            1 => directory_column("comment", "Комментарий", 320.0, false),
            _ => directory_column("actions", "Действия", ACTIONS_WIDTH, true),
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(selectable_text(
                "expense-places-empty",
                "Места покупок не найдены.",
            ))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let place = self.rows[row_ix].clone();
        let content = match col_ix {
            0 => div()
                .w_full()
                .truncate()
                .child(selectable_table_cell(
                    "expense-places",
                    place.id,
                    col_ix,
                    place.name.clone(),
                ))
                .into_any_element(),
            1 => match place.comment.as_deref() {
                Some(comment) if !comment.is_empty() => {
                    let full_comment = comment.to_owned();
                    let comment_id = place.id;
                    div()
                        .id(("expense-place-comment", row_ix))
                        .w_full()
                        .truncate()
                        .cursor_pointer()
                        .child(selectable_table_cell(
                            "expense-places",
                            comment_id,
                            col_ix,
                            comment_preview(comment),
                        ))
                        .on_click(move |_, window, cx| {
                            let comment = full_comment.clone();
                            window.open_dialog(cx, move |dialog: Dialog, _, _| {
                                dialog.title("Комментарий").centered().child(
                                    div()
                                        .id(("expense-place-comment-dialog", comment_id as usize))
                                        .max_h(px(420.0))
                                        .overflow_y_scroll()
                                        .overflow_x_scroll()
                                        .child(selectable_text(
                                            ("expense-place-comment-dialog", comment_id as usize),
                                            comment.clone(),
                                        )),
                                )
                            });
                        })
                        .into_any_element()
                }
                _ => div().w_full().into_any_element(),
            },
            _ => {
                let id = place.id;
                let edit_page = self.page.clone();
                let edit = place.clone();
                let delete_page = self.page.clone();
                let subject = format!("место покупок «{}»", place.name);
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .child(
                        Button::new(("expense-place-edit", row_ix))
                            .xsmall()
                            .label("Изменить")
                            .on_click(move |_, window, cx| {
                                let edit = edit.clone();
                                let _ = edit_page.update(cx, |page, cx| {
                                    page.open_expense_place_form(Some(&edit), window, cx)
                                });
                            }),
                    )
                    .child(
                        Button::new(("expense-place-delete", row_ix))
                            .danger()
                            .xsmall()
                            .label("Удалить")
                            .on_click(move |_, window, cx| {
                                let subject = subject.clone();
                                let _ = delete_page.update(cx, |page, cx| {
                                    page.confirm_delete(
                                        Delete::ExpensePlace(id),
                                        subject,
                                        window,
                                        cx,
                                    )
                                });
                            }),
                    )
                    .into_any_element()
            }
        };
        directory_cell(content, col_ix, cx)
    }

    fn cell_label(&self, row_ix: usize, col_ix: usize, _: &App) -> Option<SharedString> {
        let place = &self.rows[row_ix];
        Some(
            match col_ix {
                0 => place.name.clone(),
                1 => place.comment.clone().unwrap_or_default(),
                _ => "Действия".to_owned(),
            }
            .into(),
        )
    }
}

impl TableDelegate for CurrenciesTable {
    fn columns_count(&self, _: &App) -> usize {
        4
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let widths = currency_column_widths();
        match col_ix {
            0 => directory_column("currency", "Валюта", widths[0], false),
            1 => directory_column("rate", "Курс", widths[1], false),
            2 => directory_column("primary", "Основная", widths[2], false),
            _ => directory_column("actions", "Действия", widths[3], true),
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(selectable_text("currencies-empty", "Валюты не найдены."))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = self.rows[row_ix].clone();
        let content = match col_ix {
            0 => div()
                .w_full()
                .truncate()
                .child(selectable_table_cell(
                    "currencies",
                    row.currency.code.clone(),
                    col_ix,
                    currency_cell_label(&row.currency),
                ))
                .into_any_element(),
            1 => {
                let tooltip = row.rate_label.clone();
                div()
                    .id(("currency-rate-label", row_ix))
                    .w_full()
                    .truncate()
                    .text_color(cx.theme().muted_foreground)
                    .child(selectable_table_cell(
                        "currencies",
                        row.currency.code.clone(),
                        col_ix,
                        row.rate_label.clone(),
                    ))
                    .tooltip(move |window, cx| {
                        ComponentTooltip::new(tooltip.clone()).build(window, cx)
                    })
                    .into_any_element()
            }
            2 => {
                if row.currency.is_primary {
                    div()
                        .w_full()
                        .flex()
                        .justify_center()
                        .text_color(cx.theme().muted_foreground)
                        .child(selectable_table_cell(
                            "currencies",
                            row.currency.code.clone(),
                            col_ix,
                            "Основная",
                        ))
                        .into_any_element()
                } else {
                    let page = self.page.clone();
                    let code = row.currency.code.clone();
                    let has_rate = row.currency.rate_to_primary.is_some();
                    let primary_tooltip = if has_rate {
                        "Сделать основной валютой"
                    } else {
                        "Сначала задайте курс этой валюты"
                    };
                    div()
                        .w_full()
                        .flex()
                        .justify_center()
                        .child(
                            Button::new(("currency-primary", row_ix))
                                .compact()
                                .label(MAKE_PRIMARY_BUTTON_LABEL)
                                .tooltip(primary_tooltip)
                                .disabled(!has_rate)
                                .on_click(move |_, window, cx| {
                                    let code = code.clone();
                                    let _ = page.update(cx, |page, cx| {
                                        page.make_primary(&code, window, cx)
                                    });
                                }),
                        )
                        .into_any_element()
                }
            }
            _ => {
                let rate_page = self.page.clone();
                let edit = row.currency.clone();
                let delete_page = self.page.clone();
                let code = row.currency.code.clone();
                let subject = format!("валюту {code}");
                let delete_code = code.clone();
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .child(
                        Button::new(("currency-rate", row_ix))
                            .xsmall()
                            .label(EDIT_CURRENCY_BUTTON_LABEL)
                            .disabled(row.currency.is_primary)
                            .on_click(move |_, window, cx| {
                                let edit = edit.clone();
                                let _ = rate_page.update(cx, |page, cx| {
                                    page.open_currency_form(Some(&edit), window, cx)
                                });
                            }),
                    )
                    .child(
                        Button::new(("currency-delete", row_ix))
                            .danger()
                            .xsmall()
                            .label("Удалить")
                            .disabled(row.currency.is_primary || row.used)
                            .tooltip(if row.currency.is_primary {
                                "Сначала выберите другую основную валюту"
                            } else {
                                "Валюта используется; сначала удалите связанные финансовые записи"
                            })
                            .on_click(move |_, window, cx| {
                                let subject = subject.clone();
                                let code = delete_code.clone();
                                let _ = delete_page.update(cx, |page, cx| {
                                    page.confirm_delete(Delete::Currency(code), subject, window, cx)
                                });
                            }),
                    )
                    .into_any_element()
            }
        };
        directory_cell(content, col_ix, cx)
    }

    fn cell_label(&self, row_ix: usize, col_ix: usize, _: &App) -> Option<SharedString> {
        let row = &self.rows[row_ix];
        Some(
            match col_ix {
                0 => currency_cell_label(&row.currency),
                1 => row.rate_label.clone(),
                2 if row.currency.is_primary => "Основная".to_owned(),
                _ => "Действия".to_owned(),
            }
            .into(),
        )
    }
}

impl Render for DirectoriesPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        let tab = self.right_tab;
        let danger = cx.theme().danger;
        let toggle = |id: &'static str, label: &'static str, active: bool| {
            let button = Button::new(id).label(label);
            if active {
                button.primary()
            } else {
                button
            }
        };
        let right = match tab {
            RightTab::Banks => self.render_banks(cx),
            RightTab::Currencies => self.render_currencies(cx),
            RightTab::ExpensePlaces => self.render_expense_places(cx),
        };
        let add_right = match tab {
            RightTab::Banks => Button::new("bank-add")
                .primary()
                .small()
                .label("Добавить банк")
                .icon(IconName::Plus)
                .on_click(cx.listener(|page, _, window, cx| page.open_bank_form(None, window, cx))),
            RightTab::Currencies => Button::new("currency-add")
                .primary()
                .small()
                .label("Добавить валюту")
                .icon(IconName::Plus)
                .on_click(
                    cx.listener(|page, _, window, cx| page.open_currency_form(None, window, cx)),
                ),
            RightTab::ExpensePlaces => Button::new("expense-place-add")
                .primary()
                .small()
                .label("Добавить место")
                .icon(IconName::Plus)
                .on_click(cx.listener(|page, _, window, cx| {
                    page.open_expense_place_form(None, window, cx)
                })),
        };
        div()
            .id("directories-page")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .child(
                div()
                    .flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(24.0))
                            .font_weight(FontWeight::BOLD)
                            .child("Справочники"),
                    )
                    .child(
                        Button::new("refresh-directories")
                            .secondary()
                            .large()
                            .label("Обновить")
                            .icon(IconName::Replace)
                            .on_click(cx.listener(|page, _, window, cx| page.reload(window, cx))),
                    ),
            )
            .when_some(self.message.clone(), |root, (message, _)| {
                root.child(selectable_message(
                    "directories-page-error",
                    message,
                    danger,
                ))
            })
            .child(
                div()
                    .flex()
                    .gap_6()
                    .items_start()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .children(CATEGORY_KINDS.into_iter().map(|item_kind| {
                                        toggle(
                                            category_kind_toggle_id(item_kind),
                                            category_kind_toggle_label(item_kind),
                                            item_kind == kind,
                                        )
                                        .on_click(
                                            cx.listener(move |page, _, window, cx| {
                                                page.set_kind(item_kind, window, cx)
                                            }),
                                        )
                                    })),
                            )
                            .child(self.render_categories(cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().flex().items_center().gap_2().children(
                                RightTab::ALL.into_iter().map(|item| {
                                    toggle(item.toggle_id(), item.title(), item == tab).on_click(
                                        cx.listener(move |page, _, _, cx| {
                                            page.set_right_tab(item, cx)
                                        }),
                                    )
                                }),
                            ))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .font_weight(FontWeight::BOLD)
                                            .child(tab.title()),
                                    )
                                    .child(add_right),
                            )
                            .child(right),
                    ),
            )
            .child(self.prices.clone().into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        currency_cell_label, currency_column_widths, directory_column, parent_category_options,
        Category, RightTab, ACTIONS_WIDTH, BANK_NAME_COLUMN_WIDTH, CATEGORY_KINDS,
        DEFAULT_CATEGORY_KIND, EDIT_CURRENCY_BUTTON_LABEL, MAKE_PRIMARY_BUTTON_LABEL, TABLE_HEIGHT,
        TABLE_ROW_HEIGHT, VISIBLE_TABLE_ROWS,
    };
    use finance_core::CurrencyDefinition;
    use finance_core::TransactionKind;
    use gpui_kit::component::table::ColumnFixed;

    fn category(id: i64, parent_id: Option<i64>, name: &str) -> Category {
        Category {
            id,
            name: name.to_owned(),
            parent_id,
            is_active: true,
        }
    }

    fn categories() -> Vec<Category> {
        vec![
            category(2, None, "Транспорт"),
            category(3, Some(2), "Разное"),
            category(1, None, "Зарплата"),
            category(4, Some(1), "Премия"),
        ]
    }

    #[test]
    fn parent_options_use_the_same_tree_paths_as_transaction_filters() {
        let options = parent_category_options(&categories(), None);

        assert_eq!(
            options,
            vec![
                (None, "Нет".to_owned()),
                (Some(2), "Транспорт".to_owned()),
                (Some(3), "Транспорт > Разное".to_owned()),
                (Some(1), "Зарплата".to_owned()),
                (Some(4), "Зарплата > Премия".to_owned()),
            ]
        );
    }

    #[test]
    fn parent_options_exclude_edited_category_and_its_descendants() {
        let options = parent_category_options(&categories(), Some(2));

        assert_eq!(
            options,
            vec![
                (None, "Нет".to_owned()),
                (Some(1), "Зарплата".to_owned()),
                (Some(4), "Зарплата > Премия".to_owned()),
            ]
        );

        let options = parent_category_options(&categories(), Some(3));
        assert!(options.contains(&(Some(2), "Транспорт".to_owned())));
        assert!(!options.iter().any(|(id, _)| *id == Some(3)));
    }

    #[test]
    fn builtin_currency_cells_show_only_the_code() {
        let builtin = |code: &str, name: &str| CurrencyDefinition {
            code: code.to_owned(),
            name: name.to_owned(),
            is_primary: code == "RUB",
            rate_to_primary: None,
            rate_date: None,
        };

        assert_eq!(
            currency_cell_label(&builtin("RUB", "Российский рубль")),
            "RUB"
        );
        assert_eq!(currency_cell_label(&builtin("USD", "Доллар США")), "USD");
        assert_eq!(currency_cell_label(&builtin("EUR", "Евро")), "EUR Евро");
    }

    #[test]
    fn currency_columns_match_bank_name_width() {
        let widths = currency_column_widths();

        assert_eq!(widths, [80.0, 160.0, 240.0, 160.0]);
        assert_eq!(widths.iter().sum::<f32>(), 640.0);
        assert_eq!(widths[0] + widths[1] + widths[2], BANK_NAME_COLUMN_WIDTH);
        assert_eq!(
            widths.iter().sum::<f32>(),
            BANK_NAME_COLUMN_WIDTH + ACTIONS_WIDTH
        );
        // Ширина колонки «Основная» должна вмещать кнопку «Назначить основной»,
        // иначе её подпись обрезается многоточием.
        assert!(widths[2] >= 180.0);
    }

    #[test]
    fn currency_row_buttons_use_the_requested_labels() {
        assert_eq!(MAKE_PRIMARY_BUTTON_LABEL, "Назначить основной");
        assert_eq!(EDIT_CURRENCY_BUTTON_LABEL, "Изменить");
    }

    #[test]
    fn directory_table_height_contains_ten_rows_and_a_header() {
        assert_eq!(VISIBLE_TABLE_ROWS, 10);
        assert_eq!(TABLE_HEIGHT, TABLE_ROW_HEIGHT * 11.0);
    }

    #[test]
    fn directory_action_columns_are_fixed_at_the_right_edge() {
        let actions = directory_column("actions", "Действия", ACTIONS_WIDTH, true);
        assert_eq!(actions.fixed, Some(ColumnFixed::Right));
        assert_eq!(f32::from(actions.width), ACTIONS_WIDTH);
        assert!(!actions.selectable);

        let name = directory_column("name", "Название", BANK_NAME_COLUMN_WIDTH, false);
        assert_eq!(name.fixed, None);
        assert!(name.selectable);
    }

    #[test]
    fn expenses_are_the_first_and_default_category_section() {
        assert_eq!(DEFAULT_CATEGORY_KIND, TransactionKind::Expense);
        assert_eq!(
            CATEGORY_KINDS,
            [TransactionKind::Expense, TransactionKind::Income]
        );
    }

    #[test]
    fn right_directory_tabs_keep_places_before_currencies() {
        assert_eq!(
            RightTab::ALL,
            [
                RightTab::Banks,
                RightTab::ExpensePlaces,
                RightTab::Currencies
            ]
        );
        let titles: Vec<_> = RightTab::ALL.iter().map(|tab| tab.title()).collect();
        assert_eq!(titles, ["Банки", "Места покупок", "Валюты"]);
    }
}
