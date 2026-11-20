//! Страница «Справочники» на `gpui-kit`: категории доходов и расходов, банки
//! и валюты. Правила защиты используемых записей остаются в `finance-db`.

use std::collections::HashSet;
use std::rc::Rc;
use std::str::FromStr;

use chrono::{Local, NaiveDate};
use finance_core::{Bank, Category, CurrencyDefinition, TransactionKind};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{AlertDialog, Dialog};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rust_decimal::Decimal;

use super::support::category_tree_rows;
use super::support::currency_rate_label;

const DATE_FORMAT: &str = "%Y-%m-%d";
const ROW_HEIGHT: f32 = 36.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RightTab {
    Banks,
    Currencies,
}

struct CategoryForm {
    id: Option<i64>,
    name: Entity<InputState>,
    parent_ids: Vec<Option<i64>>,
    parent_id: Option<i64>,
    parent_select: Entity<SelectState<Vec<SharedString>>>,
    is_active: bool,
}

struct BankForm {
    id: Option<i64>,
    name: Entity<InputState>,
    comment: Entity<InputState>,
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
}

enum Delete {
    Category(i64),
    Bank(i64),
    Currency(String),
}

pub(crate) struct DirectoriesPage {
    database: Rc<Database>,
    kind: TransactionKind,
    right_tab: RightTab,
    categories: Vec<Category>,
    banks: Vec<Bank>,
    currencies: Vec<CurrencyDefinition>,
    used_currencies: HashSet<String>,
    message: Option<(String, bool)>,
    category_form: Option<CategoryForm>,
    bank_form: Option<BankForm>,
    currency_form: Option<CurrencyForm>,
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

impl DirectoriesPage {
    pub(crate) fn new(database: Rc<Database>, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            database,
            kind: TransactionKind::Income,
            right_tab: RightTab::Banks,
            categories: Vec::new(),
            banks: Vec::new(),
            currencies: Vec::new(),
            used_currencies: HashSet::new(),
            message: None,
            category_form: None,
            bank_form: None,
            currency_form: None,
            _subscriptions: Vec::new(),
        };
        page.reload(cx);
        page
    }

    fn error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.message = Some((message.into(), true));
        cx.notify();
    }

    /// Перечитывает все справочники; ошибки показываются, данные не подменяются пустыми.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        let db = &self.database;
        let categories = finance_db::list_categories(db, self.kind, true);
        let banks = finance_db::list_banks(db);
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
    }

    fn set_kind(&mut self, kind: TransactionKind, cx: &mut Context<Self>) {
        if self.kind != kind {
            self.kind = kind;
            self.category_form = None;
            self.reload(cx);
        }
    }

    fn set_right_tab(&mut self, tab: RightTab, cx: &mut Context<Self>) {
        if self.right_tab != tab {
            self.right_tab = tab;
            self.bank_form = None;
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
        let parents: Vec<&Category> = self
            .categories
            .iter()
            .filter(|c| Some(c.id) != own_id)
            .collect();
        let parent_ids: Vec<Option<i64>> = std::iter::once(None)
            .chain(parents.iter().map(|c| Some(c.id)))
            .collect();
        let items: Vec<SharedString> = std::iter::once("Нет".to_owned())
            .chain(parents.iter().map(|c| c.name.clone()))
            .map(SharedString::from)
            .collect();
        let parent_id = existing.and_then(|c| c.parent_id);
        let selected = parent_ids.iter().position(|p| *p == parent_id).unwrap_or(0);
        let parent_select = cx.new(|cx| {
            SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx)
        });
        let sub = cx.subscribe_in(
            &parent_select,
            window,
            |this: &mut Self, state, event: &SelectEvent<Vec<SharedString>>, _, cx| {
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
                self.reload(cx);
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
                self.reload(cx);
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
                self.reload(cx);
            }
            Err(error) => self.error(error.to_string(), cx),
        }
    }

    fn make_primary(&mut self, code: &str, cx: &mut Context<Self>) {
        match finance_db::set_primary_currency(&self.database, code) {
            Ok(()) => self.reload(cx),
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
            };
            alert
                .title("Подтверждение удаления")
                .description(description.clone())
                .ok_text("Удалить")
                .cancel_text("Отмена")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    this.update(cx, |page, cx| {
                        let result = match &target {
                            Delete::Category(id) => {
                                finance_db::delete_category(&page.database, page.kind, *id)
                            }
                            Delete::Bank(id) => finance_db::delete_bank(&page.database, *id),
                            Delete::Currency(code) => {
                                finance_db::delete_currency(&page.database, code)
                            }
                        };
                        match result {
                            Ok(()) => page.reload(cx),
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
                let title = if form.id.is_some() {
                    "Изменить категорию"
                } else {
                    "Новая категория"
                };
                let fields = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child("Название")
                    .child(Input::new(&form.name))
                    .child("Родительская категория")
                    .child(Select::new(&form.parent_select))
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
                body.child(div().text_color(danger).child(message))
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
                                FormKind::Currency => page.save_currency(window, cx),
                            })),
                    ),
            );
        (title, body.into_any_element())
    }

    // ---- отрисовка ----

    fn list(&self, has_rows: bool, cx: &App) -> Div {
        div().flex().flex_col().when(has_rows, |list| {
            list.border_1().border_color(cx.theme().border)
        })
    }

    fn row(&self, id: impl Into<ElementId>, index: usize, cx: &App) -> Stateful<Div> {
        div()
            .id(id)
            .flex()
            .h(px(ROW_HEIGHT))
            .items_center()
            .when(index > 0, |row| {
                row.border_t_1().border_color(cx.theme().border)
            })
            .when(index % 2 == 1, |row| row.bg(cx.theme().muted))
    }

    fn action_cell(&self, cx: &App) -> Div {
        div()
            .flex()
            .flex_none()
            .h_full()
            .items_center()
            .border_l_1()
            .border_color(cx.theme().border)
            .px_1()
    }

    fn render_categories(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = category_tree_rows(&self.categories);
        let mut list = self.list(!rows.is_empty(), cx);
        for (position, (index, depth)) in rows.into_iter().enumerate() {
            let category = self.categories[index].clone();
            let id = category.id;
            let prefix = if depth == 0 {
                String::new()
            } else {
                format!("{}> ", "  ".repeat(depth))
            };
            let suffix = if category.is_active {
                ""
            } else {
                " (неактивна)"
            };
            let label = format!("{prefix}{}{suffix}", category.name);
            let subject = format!("категорию «{}»", category.name);
            let edit = category.clone();
            list = list.child(
                self.row(("category", id as usize), position, cx)
                    .child(div().flex_1().px_2().child(label))
                    .child(
                        self.action_cell(cx).child(
                            Button::new(("category-edit", id as usize))
                                .xsmall()
                                .label("Изменить")
                                .on_click(cx.listener(move |page, _, window, cx| {
                                    page.open_category_form(Some(&edit), window, cx)
                                })),
                        ),
                    )
                    .child(
                        self.action_cell(cx).child(
                            Button::new(("category-delete", id as usize))
                                .danger()
                                .xsmall()
                                .label("Удалить")
                                .on_click(cx.listener(move |page, _, window, cx| {
                                    page.confirm_delete(
                                        Delete::Category(id),
                                        subject.clone(),
                                        window,
                                        cx,
                                    )
                                })),
                        ),
                    ),
            );
        }
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
            .child(list)
    }

    fn render_banks(&mut self, cx: &mut Context<Self>) -> Div {
        let mut list = self.list(!self.banks.is_empty(), cx);
        for (position, bank) in self.banks.iter().cloned().enumerate() {
            let id = bank.id;
            let subject = format!("банк «{}»", bank.name);
            let edit = bank.clone();
            list = list.child(
                self.row(("bank", id as usize), position, cx)
                    .child(div().flex_1().px_2().child(bank.name.clone()))
                    .child(
                        self.action_cell(cx).child(
                            Button::new(("bank-edit", id as usize))
                                .xsmall()
                                .label("Изменить")
                                .on_click(cx.listener(move |page, _, window, cx| {
                                    page.open_bank_form(Some(&edit), window, cx)
                                })),
                        ),
                    )
                    .child(
                        self.action_cell(cx).child(
                            Button::new(("bank-delete", id as usize))
                                .danger()
                                .xsmall()
                                .label("Удалить")
                                .on_click(cx.listener(move |page, _, window, cx| {
                                    page.confirm_delete(
                                        Delete::Bank(id),
                                        subject.clone(),
                                        window,
                                        cx,
                                    )
                                })),
                        ),
                    ),
            );
        }
        div().flex().flex_col().gap_2().child(list)
    }

    fn render_currencies(&mut self, cx: &mut Context<Self>) -> Div {
        let primary_code = self
            .currencies
            .iter()
            .find(|c| c.is_primary)
            .map_or("—".to_owned(), |c| c.code.clone());
        let muted = cx.theme().muted_foreground;
        let mut list = self.list(!self.currencies.is_empty(), cx);
        for (position, currency) in self.currencies.iter().cloned().enumerate() {
            let code = currency.code.clone();
            let used = self.used_currencies.contains(&code);
            let rate_label = currency_rate_label(&currency, &primary_code);
            let subject = format!("валюту {code}");
            let edit = currency.clone();
            let (c1, c2, c3) = (code.clone(), code.clone(), code.clone());
            list = list.child(
                self.row(("currency", position), position, cx)
                    .h(px(ROW_HEIGHT + 12.0))
                    .child(
                        div()
                            .flex_1()
                            .px_2()
                            .child(div().child(format!("{} {}", currency.code, currency.name)))
                            .child(div().text_color(muted).child(rate_label)),
                    )
                    .child(
                        self.action_cell(cx).child(Button::new(("currency-rate", position))
                            .xsmall()
                            .label("Изменить курс")
                            .disabled(currency.is_primary)
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.open_currency_form(Some(&edit), window, cx)
                            }))),
                    )
                    .child(
                        self.action_cell(cx)
                            .w(px(152.0))
                            .justify_center()
                            .child(if currency.is_primary {
                                div().text_color(muted).child("Основная")
                            } else {
                                div().child(
                                    Button::new(("currency-primary", position))
                                        .compact()
                                        .label("Сделать основной")
                                        .tooltip("Сначала задайте курс этой валюты")
                                        .disabled(currency.rate_to_primary.is_none())
                                        .on_click(cx.listener(move |page, _, _, cx| {
                                            page.make_primary(&c1, cx)
                                        })),
                                )
                            }),
                    )
                    .child(
                        self.action_cell(cx).child(Button::new(("currency-delete", position))
                            .danger()
                            .xsmall()
                            .label("Удалить")
                            .disabled(currency.is_primary || used)
                            .tooltip(if currency.is_primary {
                                "Сначала выберите другую основную валюту"
                            } else {
                                "Валюта используется; сначала удалите связанные финансовые записи"
                            })
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.confirm_delete(
                                    Delete::Currency(c2.clone()),
                                    subject.clone(),
                                    window,
                                    cx,
                                )
                            }))),
                    ),
            );
            let _ = &c3;
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(list)
            .child(div().text_color(muted).child(
                "Курс означает количество единиц основной валюты за 1 единицу выбранной валюты.",
            ))
            .child(div().text_color(muted).child(
                "Дополнительные валюты пока нельзя выбирать в формах доходов и расходов (поддерживаются RUB и USD).",
            ))
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
                            .on_click(cx.listener(|page, _, _, cx| page.reload(cx))),
                    ),
            )
            .when_some(self.message.clone(), |root, (message, _)| {
                root.child(div().text_color(danger).child(message))
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
                                    .child(
                                        toggle(
                                            "kind-income",
                                            "Категории доходов",
                                            kind == TransactionKind::Income,
                                        )
                                        .on_click(
                                            cx.listener(|page, _, _, cx| {
                                                page.set_kind(TransactionKind::Income, cx)
                                            }),
                                        ),
                                    )
                                    .child(
                                        toggle(
                                            "kind-expense",
                                            "Категории расходов",
                                            kind == TransactionKind::Expense,
                                        )
                                        .on_click(
                                            cx.listener(|page, _, _, cx| {
                                                page.set_kind(TransactionKind::Expense, cx)
                                            }),
                                        ),
                                    ),
                            )
                            .child(self.render_categories(cx)),
                    )
                    .child(
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
                                        toggle("tab-banks", "Банки", tab == RightTab::Banks)
                                            .on_click(cx.listener(|page, _, _, cx| {
                                                page.set_right_tab(RightTab::Banks, cx)
                                            })),
                                    )
                                    .child(
                                        toggle(
                                            "tab-currencies",
                                            "Валюты",
                                            tab == RightTab::Currencies,
                                        )
                                        .on_click(
                                            cx.listener(|page, _, _, cx| {
                                                page.set_right_tab(RightTab::Currencies, cx)
                                            }),
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().flex_1().font_weight(FontWeight::BOLD).child(
                                        match tab {
                                            RightTab::Banks => "Банки",
                                            RightTab::Currencies => "Валюты",
                                        },
                                    ))
                                    .child(add_right),
                            )
                            .child(right),
                    ),
            )
    }
}
