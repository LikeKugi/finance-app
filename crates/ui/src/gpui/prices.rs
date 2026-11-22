//! Секция управления ценами для страницы «Справочники».

use std::cmp::Ordering;
use std::rc::Rc;
use std::str::FromStr;

use chrono::{Local, NaiveDate};
use finance_core::{Currency, ExpensePlace, PriceFilter, PriceRecord};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{AlertDialog, Dialog};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::table::{
    Column, ColumnFixed, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
};
use gpui_kit::component::{ActiveTheme as _, IconName, IndexPath, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rust_decimal::Decimal;

const DATE_FORMAT: &str = "%Y-%m-%d";
const WIDTHS_KEY: &str = "table_columns.prices.v1";
const HEADERS: [&str; 6] = [
    "Название",
    "Дата",
    "Цена",
    "Валюта",
    "Место покупки",
    "Действия",
];
const DEFAULT_WIDTHS: [f32; 6] = [220.0, 120.0, 140.0, 100.0, 190.0, 170.0];
const MIN_WIDTH: f32 = 70.0;
const MAX_WIDTH: f32 = 600.0;
const ACTIONS_WIDTH: f32 = 170.0;

type Choice = Entity<SelectState<Vec<SharedString>>>;

struct PriceForm {
    id: Option<i64>,
    name: Entity<InputState>,
    date: Entity<InputState>,
    amount: Entity<InputState>,
    currency: Choice,
    place: Choice,
    place_ids: Vec<i64>,
}

pub(crate) struct PricesSection {
    database: Rc<Database>,
    places: Vec<ExpensePlace>,
    entries: Vec<PriceRecord>,
    table: Entity<TableState<PricesTable>>,
    currency_filter: Choice,
    place_filter: Choice,
    date_from: Entity<InputState>,
    date_to: Entity<InputState>,
    filter_currency: Option<Currency>,
    filter_place: Option<i64>,
    form: Option<PriceForm>,
    message: Option<String>,
    _subscriptions: Vec<Subscription>,
}

struct PricesTable {
    page: WeakEntity<PricesSection>,
    rows: Vec<PriceRecord>,
    widths: Vec<f32>,
    sort: Option<(usize, ColumnSort)>,
}

fn input(placeholder: &str, value: &str, window: &mut Window, cx: &mut App) -> Entity<InputState> {
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

fn choice(items: Vec<String>, selected: usize, window: &mut Window, cx: &mut App) -> Choice {
    let items = items.into_iter().map(SharedString::from).collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx))
}

fn selected_row(state: &Choice, cx: &App) -> Option<usize> {
    state.read(cx).selected_index(cx).map(|index| index.row)
}

fn sort_prices(rows: &mut [PriceRecord], sort: Option<(usize, ColumnSort)>) {
    let Some((column, direction)) = sort else {
        rows.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        return;
    };
    rows.sort_by(|a, b| {
        let ordering = match column {
            0 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            1 => a.date.cmp(&b.date),
            2 => a.amount.cmp(&b.amount),
            3 => a.currency.code().cmp(b.currency.code()),
            4 => a
                .expense_place_name
                .to_lowercase()
                .cmp(&b.expense_place_name.to_lowercase()),
            _ => Ordering::Equal,
        };
        let ordering = if direction == ColumnSort::Descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then(a.id.cmp(&b.id))
    });
}

impl PricesSection {
    pub(crate) fn new(database: Rc<Database>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let currency_filter = choice(
            vec!["Все валюты".to_owned(), "RUB".to_owned(), "USD".to_owned()],
            0,
            window,
            cx,
        );
        let place_filter = choice(vec!["Все места".to_owned()], 0, window, cx);
        let date_from = input("ГГГГ-ММ-ДД", "", window, cx);
        let date_to = input("ГГГГ-ММ-ДД", "", window, cx);
        let widths = load_widths(&database);
        let page = cx.entity().downgrade();
        let table = cx.new(|cx| {
            TableState::new(
                PricesTable {
                    page: page.clone(),
                    rows: Vec::new(),
                    widths,
                    sort: None,
                },
                window,
                cx,
            )
            .col_movable(false)
        });
        let mut subscriptions = vec![
            cx.subscribe_in(
                &currency_filter,
                window,
                |page: &mut Self, state, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                    page.filter_currency = selected_row(state, cx).and_then(|row| match row {
                        1 => Some(Currency::Rub),
                        2 => Some(Currency::Usd),
                        _ => None,
                    });
                    page.reload(cx);
                },
            ),
            cx.subscribe_in(
                &place_filter,
                window,
                |page: &mut Self, state, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                    page.filter_place = selected_row(state, cx)
                        .and_then(|row| row.checked_sub(1))
                        .and_then(|row| page.places.get(row))
                        .map(|place| place.id);
                    page.reload(cx);
                },
            ),
        ];
        subscriptions.push(
            cx.subscribe(&table, |page: &mut Self, _, event: &TableEvent, cx| {
                if let TableEvent::ColumnWidthsChanged(widths) = event {
                    page.save_widths(widths, cx);
                }
            }),
        );
        let mut page = Self {
            database,
            places: Vec::new(),
            entries: Vec::new(),
            table,
            currency_filter,
            place_filter,
            date_from,
            date_to,
            filter_currency: None,
            filter_place: None,
            form: None,
            message: None,
            _subscriptions: subscriptions,
        };
        page.reload_places(window, cx);
        page
    }

    fn error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.message = Some(message.into());
        cx.notify();
    }

    pub(crate) fn reload_places(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match finance_db::list_expense_places(&self.database) {
            Ok(places) => {
                self.places = places;
                let mut labels = vec![SharedString::from("Все места")];
                labels.extend(
                    self.places
                        .iter()
                        .map(|place| SharedString::from(place.name.clone())),
                );
                self.place_filter.update(cx, |state, cx| {
                    state.set_items(labels, window, cx);
                    let selected = self
                        .filter_place
                        .and_then(|id| self.places.iter().position(|place| place.id == id))
                        .map_or(0, |row| row + 1);
                    state.set_selected_index(Some(IndexPath::default().row(selected)), window, cx);
                });
                self.reload(cx);
            }
            Err(error) => self.error(error.to_string(), cx),
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let date_from_text = self.date_from.read(cx).value().to_string();
        let date_to_text = self.date_to.read(cx).value().to_string();
        let parse_date = |value: &str| -> Result<Option<NaiveDate>, String> {
            if value.trim().is_empty() {
                Ok(None)
            } else {
                NaiveDate::parse_from_str(value.trim(), DATE_FORMAT)
                    .map(Some)
                    .map_err(|_| "Введите дату в формате ГГГГ-ММ-ДД".to_owned())
            }
        };
        let (date_from, date_to) = match (parse_date(&date_from_text), parse_date(&date_to_text)) {
            (Ok(from), Ok(to)) => (from, to),
            (Err(error), _) | (_, Err(error)) => {
                self.error(error, cx);
                return;
            }
        };
        let filter = PriceFilter {
            currency: self.filter_currency,
            expense_place_id: self.filter_place,
            date_from,
            date_to,
        };
        match finance_db::list_prices(&self.database, filter) {
            Ok(rows) => {
                self.entries = rows;
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_rows(self.entries.clone());
                    cx.notify();
                });
                self.message = None;
                cx.notify();
            }
            Err(error) => self.error(error.to_string(), cx),
        }
    }

    fn apply_dates(&mut self, cx: &mut Context<Self>) {
        self.reload(cx);
    }

    fn open_form(
        &mut self,
        record: Option<&PriceRecord>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.places.is_empty() {
            self.error("Сначала добавьте место покупки в справочнике", cx);
            return;
        }
        let date = record.map_or_else(|| Local::now().date_naive(), |entry| entry.date);
        let currency = record.map_or(0, |entry| {
            if entry.currency == Currency::Rub {
                0
            } else {
                1
            }
        });
        let place_index = record
            .and_then(|entry| {
                self.places
                    .iter()
                    .position(|place| place.id == entry.expense_place_id)
            })
            .unwrap_or(0);
        let place_ids = self.places.iter().map(|place| place.id).collect();
        self.form = Some(PriceForm {
            id: record.map(|entry| entry.id),
            name: input(
                "Название товара или услуги",
                record.map_or("", |entry| entry.name.as_str()),
                window,
                cx,
            ),
            date: input(
                "ГГГГ-ММ-ДД",
                &date.format(DATE_FORMAT).to_string(),
                window,
                cx,
            ),
            amount: input(
                "Цена за 1 шт. / 1 кг",
                &record.map_or_else(String::new, |entry| entry.amount.to_string()),
                window,
                cx,
            ),
            currency: choice(
                vec!["RUB".to_owned(), "USD".to_owned()],
                currency,
                window,
                cx,
            ),
            place: choice(
                self.places.iter().map(|place| place.name.clone()).collect(),
                place_index,
                window,
                cx,
            ),
            place_ids,
        });
        let this = cx.entity();
        window.open_dialog(cx, move |dialog: Dialog, _, cx| {
            let (title, body) = this.update(cx, |page, cx| page.form_content(cx));
            dialog
                .title(title)
                .child(body)
                .close_button(false)
                .overlay_closable(false)
                .keyboard(false)
        });
    }

    fn save_form(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(form) = self.form.as_ref() else {
            return false;
        };
        let name = form.name.read(cx).value().trim().to_owned();
        let date_text = form.date.read(cx).value().to_string();
        let amount_text = form.amount.read(cx).value().trim().replace(',', ".");
        let Ok(date) = NaiveDate::parse_from_str(date_text.trim(), DATE_FORMAT) else {
            self.error("Введите дату в формате ГГГГ-ММ-ДД", cx);
            return false;
        };
        let Ok(amount) = Decimal::from_str(&amount_text) else {
            self.error("Введите корректную цену", cx);
            return false;
        };
        let currency = if selected_row(&form.currency, cx) == Some(1) {
            Currency::Usd
        } else {
            Currency::Rub
        };
        let Some(place_id) = selected_row(&form.place, cx)
            .and_then(|row| form.place_ids.get(row))
            .copied()
        else {
            self.error("Выберите место покупки", cx);
            return false;
        };
        match finance_db::save_price(
            &self.database,
            form.id,
            &name,
            date,
            amount,
            currency,
            place_id,
        ) {
            Ok(_) => {
                self.form = None;
                self.reload(cx);
                true
            }
            Err(error) => {
                self.error(error.to_string(), cx);
                false
            }
        }
    }

    fn form_content(&mut self, cx: &mut Context<Self>) -> (String, AnyElement) {
        let Some(form) = &self.form else {
            return ("Цена".to_owned(), div().into_any_element());
        };
        let title = if form.id.is_some() {
            "Изменить цену"
        } else {
            "Новая цена"
        }
        .to_owned();
        let this = cx.entity();
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child("Название товара или услуги")
            .child(Input::new(&form.name))
            .child("Дата (ГГГГ-ММ-ДД)")
            .child(Input::new(&form.date))
            .child("Цена за 1 шт. / 1 кг")
            .child(Input::new(&form.amount))
            .child("Валюта")
            .child(Select::new(&form.currency))
            .child("Место покупки")
            .child(Select::new(&form.place))
            .when_some(self.message.clone(), |body, message| {
                body.child(div().text_color(cx.theme().danger).child(message))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("price-cancel")
                            .label("Отмена")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("price-save")
                            .primary()
                            .label("Сохранить")
                            .on_click(move |_, window, cx| {
                                let saved = this.update(cx, |page, cx| page.save_form(cx));
                                if saved {
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            );
        (title, body.into_any_element())
    }

    fn confirm_delete(&mut self, record: PriceRecord, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity();
        let description = format!("Удалить запись о цене «{}»?", record.name);
        window.open_alert_dialog(cx, move |alert: AlertDialog, _, _| {
            let this = this.clone();
            alert
                .title("Подтверждение удаления")
                .description(description.clone())
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    this.update(cx, |page, cx| {
                        match finance_db::delete_price(&page.database, record.id) {
                            Ok(()) => page.reload(cx),
                            Err(error) => page.error(error.to_string(), cx),
                        }
                    });
                    true
                })
        });
    }

    fn save_widths(&mut self, widths: &[Pixels], cx: &mut Context<Self>) {
        let mut widths = widths
            .iter()
            .map(|width| f32::from(*width))
            .collect::<Vec<_>>();
        if widths.len() == HEADERS.len() {
            widths[5] = ACTIONS_WIDTH;
        }
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_widths(&widths);
            cx.notify();
        });
        if let Err(error) =
            finance_db::set_app_setting(&self.database, WIDTHS_KEY, &format_widths(&widths))
        {
            eprintln!("Не удалось сохранить ширину колонок цен: {error}");
        }
    }
}

impl PricesTable {
    fn set_rows(&mut self, rows: Vec<PriceRecord>) {
        self.rows = rows;
        sort_prices(&mut self.rows, self.sort);
    }
    fn set_widths(&mut self, widths: &[f32]) {
        if let Some(widths) = widths_with_fixed_actions(widths) {
            self.widths = widths;
        }
    }
}

fn widths_with_fixed_actions(widths: &[f32]) -> Option<Vec<f32>> {
    if widths.len() != HEADERS.len() {
        return None;
    }
    let mut widths = widths.to_vec();
    widths[5] = ACTIONS_WIDTH;
    Some(widths)
}

impl TableDelegate for PricesTable {
    fn columns_count(&self, _: &App) -> usize {
        HEADERS.len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }
    fn column(&self, col_ix: usize, _: &App) -> Column {
        let actions = col_ix == 5;
        let mut column = Column::new(HEADERS[col_ix], HEADERS[col_ix])
            .width(px(self.widths[col_ix]))
            .min_width(px(if actions { ACTIONS_WIDTH } else { MIN_WIDTH }))
            .max_width(px(if actions { ACTIONS_WIDTH } else { MAX_WIDTH }))
            .resizable(!actions)
            .movable(false);
        if actions {
            column = column.fixed(ColumnFixed::Right);
        }
        if col_ix < 5 {
            column.sort(
                self.sort
                    .filter(|(index, _)| *index == col_ix)
                    .map_or(ColumnSort::Default, |(_, sort)| sort),
            )
        } else {
            column
        }
    }
    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        self.sort = (sort != ColumnSort::Default).then_some((col_ix, sort));
        sort_prices(&mut self.rows, self.sort);
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
            .child("Цены по выбранным условиям не найдены.")
    }
    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let record = self.rows[row_ix].clone();
        let content = match col_ix {
            0 => div().child(record.name.clone()).into_any_element(),
            1 => div()
                .child(record.date.format(DATE_FORMAT).to_string())
                .into_any_element(),
            2 => div()
                .w_full()
                .text_right()
                .child(record.amount.to_string())
                .into_any_element(),
            3 => div().child(record.currency.code()).into_any_element(),
            4 => div()
                .child(record.expense_place_name.clone())
                .into_any_element(),
            _ => {
                let edit_page = self.page.clone();
                let edit_record = record.clone();
                let delete_page = self.page.clone();
                let delete_record = record;
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .child(
                        Button::new(("price-edit", row_ix))
                            .xsmall()
                            .label("Изменить")
                            .on_click(move |_, window, cx| {
                                let _ = edit_page.update(cx, |page, cx| {
                                    page.open_form(Some(&edit_record), window, cx)
                                });
                            }),
                    )
                    .child(
                        Button::new(("price-delete", row_ix))
                            .danger()
                            .xsmall()
                            .label("Удалить")
                            .on_click(move |_, window, cx| {
                                let _ = delete_page.update(cx, |page, cx| {
                                    page.confirm_delete(delete_record.clone(), window, cx)
                                });
                            }),
                    )
                    .into_any_element()
            }
        };
        let border = cx.theme().border;
        div()
            .w_full()
            .h_full()
            .when(col_ix > 0, |cell| cell.border_l_1().border_color(border))
            .child(content)
            .into_any_element()
    }
}

impl Render for PricesSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let message = self.message.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(20.0))
                            .font_weight(FontWeight::BOLD)
                            .child("Цены"),
                    )
                    .child(
                        Button::new("price-add")
                            .primary()
                            .small()
                            .label("Добавить цену")
                            .icon(IconName::Plus)
                            .on_click(
                                cx.listener(|page, _, window, cx| page.open_form(None, window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(Select::new(&self.currency_filter).w(px(150.0)))
                    .child(Select::new(&self.place_filter).w(px(210.0)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child("Дата от")
                            .child(div().w(px(150.0)).child(Input::new(&self.date_from))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child("Дата до")
                            .child(div().w(px(150.0)).child(Input::new(&self.date_to))),
                    )
                    .child(
                        Button::new("prices-apply-filters")
                            .secondary()
                            .label("Применить")
                            .on_click(cx.listener(|page, _, _, cx| page.apply_dates(cx))),
                    ),
            )
            .when_some(message, |body, message| {
                body.child(div().text_color(cx.theme().danger).child(message))
            })
            .child(
                div()
                    .h(px(360.0))
                    .min_h(px(240.0))
                    .child(DataTable::new(&self.table).stripe(true).bordered(true)),
            )
    }
}

fn format_widths(widths: &[f32]) -> String {
    widths
        .iter()
        .map(|width| width.round().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn load_widths(database: &Database) -> Vec<f32> {
    match finance_db::get_app_setting(database, WIDTHS_KEY) {
        Ok(Some(value)) => {
            let parsed = value
                .split(',')
                .map(str::trim)
                .map(str::parse::<f32>)
                .collect::<Result<Vec<_>, _>>();
            if let Ok(mut widths) = parsed {
                if widths.len() == HEADERS.len()
                    && widths
                        .iter()
                        .all(|width| width.is_finite() && (MIN_WIDTH..=MAX_WIDTH).contains(width))
                {
                    widths[5] = ACTIONS_WIDTH;
                    return widths;
                }
            }
            eprintln!(
                "Некорректная сохранённая ширина колонок цен; использованы значения по умолчанию"
            );
            DEFAULT_WIDTHS.to_vec()
        }
        Ok(None) => DEFAULT_WIDTHS.to_vec(),
        Err(error) => {
            eprintln!("Не удалось прочитать ширину колонок цен: {error}");
            DEFAULT_WIDTHS.to_vec()
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use finance_core::{Currency, PriceRecord};
    use rust_decimal::Decimal;

    use super::{sort_prices, widths_with_fixed_actions, ACTIONS_WIDTH};
    use gpui_kit::component::table::ColumnSort;

    fn record(
        id: i64,
        name: &str,
        date: (i32, u32, u32),
        amount: i64,
        currency: Currency,
        place: &str,
    ) -> PriceRecord {
        PriceRecord {
            id,
            name: name.to_owned(),
            date: NaiveDate::from_ymd_opt(date.0, date.1, date.2).unwrap(),
            amount: Decimal::from(amount),
            currency,
            expense_place_id: id,
            expense_place_name: place.to_owned(),
        }
    }

    #[test]
    fn every_price_column_sorts_by_its_value() {
        let rows = vec![
            record(1, "Яблоко", (2026, 2, 1), 2, Currency::Usd, "Б"),
            record(2, "Абрикос", (2026, 1, 1), 10, Currency::Rub, "А"),
        ];
        let ascending_first = [2, 2, 1, 2, 2];
        for (column, first_id) in ascending_first.into_iter().enumerate() {
            let mut sorted = rows.clone();
            sort_prices(&mut sorted, Some((column, ColumnSort::Ascending)));
            assert_eq!(sorted[0].id, first_id, "column {column}");
            sort_prices(&mut sorted, Some((column, ColumnSort::Descending)));
            assert_eq!(
                sorted[0].id,
                if first_id == 1 { 2 } else { 1 },
                "column {column}"
            );
        }
    }

    #[test]
    fn table_width_updates_keep_actions_column_fixed() {
        let widths =
            widths_with_fixed_actions(&[300.0, 130.0, 160.0, 110.0, 250.0, 400.0]).unwrap();
        assert_eq!(widths[0], 300.0);
        assert_eq!(widths[4], 250.0);
        assert_eq!(widths[5], ACTIONS_WIDTH);
        assert!(widths_with_fixed_actions(&[100.0]).is_none());
    }
}
