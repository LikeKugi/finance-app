//! Компактные формы добавления дохода и расхода на стартовой странице.
//! Финансовые правила и запись в БД — в `finance-core` и `finance-db`.

use std::rc::Rc;

use chrono::Local;
use finance_core::{
    parse_amount_expression, Category, Currency, ExpensePlace, PaymentCardType, PaymentDocument,
    TransactionKind,
};
use finance_db::Database;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, Selectable as _, Sizable as _,
};
use gpui_kit::*;

use super::support::{
    card_label, category_tree_labels, eligible_cards, expense_place_label, quick_form_date_caption,
    DIVISION_ROUNDING_WARNING,
};

type Choice = Entity<SelectState<Vec<SharedString>>>;

const NO_CARD_LABEL: &str = "Без карты";
const NO_PLACE_LABEL: &str = "Не указано";
const AMOUNT_PLACEHOLDER: &str = "например: 100 + 20 - 5 * 2 / 4";
const CARD_FIELD_LABEL: &str = "Зачислить на карту";

fn choice(
    items: Vec<String>,
    selected: usize,
    window: &mut Window,
    cx: &mut Context<QuickTransactionForm>,
) -> Choice {
    let items: Vec<SharedString> = items.into_iter().map(SharedString::from).collect();
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx))
}

fn optional_id_by_label<T: Copy>(ids: &[Option<T>], labels: &[String], label: &str) -> Option<T> {
    labels
        .iter()
        .position(|item| item == label)
        .and_then(|index| ids.get(index).copied())
        .flatten()
}

/// Сохранение доступно только при выбранной валюте и активной категории.
fn can_save(category_id: Option<i64>, currency: Option<Currency>, has_category: bool) -> bool {
    has_category && category_id.is_some() && currency.is_some()
}

/// Тип операции выбирается ровно одной кнопкой сегментированной группы.
fn selected_kind(selected: &[usize]) -> Option<TransactionKind> {
    match selected {
        [0] => Some(TransactionKind::Income),
        [1] => Some(TransactionKind::Expense),
        _ => None,
    }
}

pub(crate) struct QuickTransactionForm {
    database: Rc<Database>,
    kind: TransactionKind,
    amount: Entity<InputState>,
    currency: Option<Currency>,
    currency_select: Choice,
    categories: Vec<Category>,
    category_ids: Vec<i64>,
    category_id: Option<i64>,
    category_select: Choice,
    cards: Vec<PaymentDocument>,
    card_ids: Vec<Option<i64>>,
    card_id: Option<i64>,
    card_select: Choice,
    places: Vec<ExpensePlace>,
    place_ids: Vec<Option<i64>>,
    place_id: Option<i64>,
    place_select: Choice,
    error: Option<String>,
    saved: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl QuickTransactionForm {
    pub(crate) fn new(database: Rc<Database>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let amount_input = cx.new(|cx| InputState::new(window, cx).placeholder(AMOUNT_PLACEHOLDER));
        let currency_select = choice(
            Currency::ALL.iter().map(|c| c.code().to_owned()).collect(),
            Currency::ALL
                .iter()
                .position(|currency| *currency == Currency::Rub)
                .unwrap_or(0),
            window,
            cx,
        );
        let category_select = choice(Vec::new(), 0, window, cx);
        let card_select = choice(vec![NO_CARD_LABEL.to_owned()], 0, window, cx);
        let place_select = choice(vec![NO_PLACE_LABEL.to_owned()], 0, window, cx);

        let subscriptions = vec![
            cx.subscribe_in(&amount_input, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &currency_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let SelectEvent::Confirm(value) = event;
                    this.currency = value.as_ref().and_then(|label| {
                        Currency::ALL
                            .into_iter()
                            .find(|currency| currency.code() == label.as_ref())
                    });
                    this.card_id = None;
                    this.rebuild_card_select(window, cx);
                },
            ),
            cx.subscribe_in(
                &category_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(label) = value {
                        let labels = category_tree_labels(&this.categories);
                        this.category_id = labels
                            .iter()
                            .find(|(id, text)| this.category_ids.contains(id) && text == label)
                            .map(|(id, _)| *id);
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &card_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(label) = value {
                        let labels = std::iter::once(NO_CARD_LABEL.to_owned())
                            .chain(
                                eligible_cards(&this.cards, this.currency)
                                    .into_iter()
                                    .map(card_label),
                            )
                            .collect::<Vec<_>>();
                        this.card_id = optional_id_by_label(&this.card_ids, &labels, label);
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &place_select,
                window,
                |this: &mut Self, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let Some(label) = value {
                        let labels = std::iter::once(NO_PLACE_LABEL.to_owned())
                            .chain(this.places.iter().map(expense_place_label))
                            .collect::<Vec<_>>();
                        this.place_id = optional_id_by_label(&this.place_ids, &labels, label);
                    }
                    cx.notify();
                },
            ),
        ];

        Self {
            database,
            kind: TransactionKind::Income,
            amount: amount_input,
            currency: Some(Currency::Rub),
            currency_select,
            categories: Vec::new(),
            category_ids: Vec::new(),
            category_id: None,
            category_select,
            cards: Vec::new(),
            card_ids: vec![None],
            card_id: None,
            card_select,
            places: Vec::new(),
            place_ids: vec![None],
            place_id: None,
            place_select,
            error: None,
            saved: None,
            _subscriptions: subscriptions,
        }
    }

    /// Переключает тип операции; доступна только одна активная кнопка.
    fn set_kind(&mut self, kind: TransactionKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.kind == kind {
            return;
        }
        self.kind = kind;
        self.category_id = None;
        self.card_id = None;
        self.error = None;
        self.saved = None;
        self.reload(window, cx);
    }

    /// Обновляет справочники при активации вкладки «Главная», не на кадре отрисовки.
    pub(crate) fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match finance_db::list_categories(&self.database, self.kind, true) {
            Ok(categories) => self.categories = categories,
            Err(error) => {
                self.fail(error.to_string(), cx);
                return;
            }
        }
        match finance_db::list_expense_places(&self.database) {
            Ok(places) => self.places = places,
            Err(error) => {
                self.fail(error.to_string(), cx);
                return;
            }
        }
        let card_type = (self.kind == TransactionKind::Income).then_some(PaymentCardType::Debit);
        match finance_db::list_payment_documents(&self.database, None, card_type, None) {
            Ok(cards) => self.cards = cards,
            Err(error) => {
                self.fail(error.to_string(), cx);
                return;
            }
        }
        self.rebuild_category_select(window, cx);
        self.rebuild_place_select(window, cx);
        self.rebuild_card_select(window, cx);
        cx.notify();
    }

    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.error = Some(message);
        self.saved = None;
        cx.notify();
    }

    fn rebuild_category_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries = category_tree_labels(&self.categories);
        self.category_ids = entries.iter().map(|(id, _)| *id).collect();
        if !self
            .category_id
            .is_some_and(|id| self.category_ids.contains(&id))
        {
            self.category_id = self.category_ids.first().copied();
        }
        let selected = self.category_id.and_then(|id| {
            self.category_ids
                .iter()
                .position(|candidate| *candidate == id)
        });
        let items: Vec<SharedString> = entries
            .into_iter()
            .map(|(_, label)| SharedString::from(label))
            .collect();
        self.category_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(
                selected.map(|row| IndexPath::default().row(row)),
                window,
                cx,
            );
        });
    }

    fn rebuild_place_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.place_ids = std::iter::once(None)
            .chain(self.places.iter().map(|place| Some(place.id)))
            .collect();
        if !self
            .place_id
            .is_some_and(|id| self.place_ids.contains(&Some(id)))
        {
            self.place_id = None;
        }
        let selected = self
            .place_id
            .and_then(|id| {
                self.place_ids
                    .iter()
                    .position(|candidate| *candidate == Some(id))
            })
            .unwrap_or(0);
        let items: Vec<SharedString> = std::iter::once(NO_PLACE_LABEL.to_owned())
            .chain(self.places.iter().map(expense_place_label))
            .map(SharedString::from)
            .collect();
        self.place_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(Some(IndexPath::default().row(selected)), window, cx);
        });
    }

    fn rebuild_card_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cards = eligible_cards(&self.cards, self.currency);
        self.card_ids = std::iter::once(None)
            .chain(cards.iter().map(|card| Some(card.id)))
            .collect();
        if !self
            .card_id
            .is_some_and(|id| self.card_ids.contains(&Some(id)))
        {
            self.card_id = None;
        }
        let selected = self
            .card_id
            .and_then(|id| {
                self.card_ids
                    .iter()
                    .position(|candidate| *candidate == Some(id))
            })
            .unwrap_or(0);
        let items: Vec<SharedString> = std::iter::once(NO_CARD_LABEL.to_owned())
            .chain(cards.iter().map(|card| card_label(card)))
            .map(SharedString::from)
            .collect();
        self.card_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(Some(IndexPath::default().row(selected)), window, cx);
        });
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let amount_text = self.amount.read(cx).value().to_string();
        let amount = match parse_amount_expression(&amount_text) {
            Ok(amount) => amount,
            Err(error) => {
                self.fail(error.to_string(), cx);
                return false;
            }
        };
        let Some(category_id) = self.category_id else {
            self.fail("Выберите категорию".to_owned(), cx);
            return false;
        };
        let Some(currency) = self.currency else {
            self.fail("Выберите валюту".to_owned(), cx);
            return false;
        };
        let result = finance_db::save_transaction_with_card_and_rate_effect_and_place(
            &self.database,
            self.kind,
            None,
            Local::now().date_naive(),
            category_id,
            amount,
            currency,
            "",
            self.card_id,
            false,
            (self.kind == TransactionKind::Expense)
                .then_some(self.place_id)
                .flatten(),
        );
        match result {
            Ok(_) => {
                self.amount
                    .update(cx, |state, cx| state.set_value(String::new(), window, cx));
                self.error = None;
                self.saved = Some(
                    match self.kind {
                        TransactionKind::Income => "Доход добавлен",
                        TransactionKind::Expense => "Расход добавлен",
                    }
                    .to_owned(),
                );
                cx.notify();
                true
            }
            Err(error) => {
                self.fail(error.to_string(), cx);
                false
            }
        }
    }
}

impl Render for QuickTransactionForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let amount_text = self.amount.read(cx).value().to_string();
        let total = (!amount_text.trim().is_empty())
            .then(|| parse_amount_expression(&amount_text).map_err(|error| error.to_string()));
        let has_category = !self.category_ids.is_empty();
        let field = |label: &'static str, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().child(label))
                .child(control)
        };

        let mut root = div()
            .w(px(480.0))
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .border_1()
            .border_color(border)
            .rounded_md()
            .child(
                div()
                    .text_size(px(18.0))
                    .font_weight(FontWeight::BOLD)
                    .child("Добавление"),
            );

        root = root
            .child(Input::new(&self.amount).large().w_full().into_any_element())
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(quick_form_date_caption(Local::now().date_naive())),
            )
            .child(
                Button::new("quick-add")
                    .large()
                    .success()
                    .label("Добавить")
                    .icon(IconName::CircleCheck)
                    .w_full()
                    .disabled(!can_save(self.category_id, self.currency, has_category))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.save(window, cx);
                    })),
            )
            .child(
                ButtonGroup::new("quick-kind")
                    .large()
                    .outline()
                    .w_full()
                    .justify_center()
                    .child(
                        Button::new("quick-kind-income")
                            .label("Доход")
                            .selected(self.kind == TransactionKind::Income),
                    )
                    .child(
                        Button::new("quick-kind-expense")
                            .label("Расход")
                            .selected(self.kind == TransactionKind::Expense),
                    )
                    .on_click(cx.listener(|this, selected: &Vec<usize>, window, cx| {
                        if let Some(kind) = selected_kind(selected) {
                            this.set_kind(kind, window, cx);
                        }
                    })),
            );

        let mut selects = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(field(
                "Валюта",
                Select::new(&self.currency_select)
                    .xsmall()
                    .w(px(120.0))
                    .into_any_element(),
            ))
            .child(field(
                "Категория",
                Select::new(&self.category_select)
                    .xsmall()
                    .w(px(240.0))
                    .into_any_element(),
            ))
            .child(field(
                CARD_FIELD_LABEL,
                Select::new(&self.card_select)
                    .xsmall()
                    .w(px(240.0))
                    .into_any_element(),
            ));
        if self.kind == TransactionKind::Expense {
            selects = selects.child(field(
                "Место траты",
                Select::new(&self.place_select)
                    .xsmall()
                    .w(px(240.0))
                    .into_any_element(),
            ));
        }
        root = root.child(selects);

        if !has_category {
            root = root.child(
                div()
                    .text_xs()
                    .text_color(danger)
                    .child("Сначала добавьте категорию в разделе «Справочники»."),
            );
        }
        if amount_text.contains('/') {
            root = root.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(DIVISION_ROUNDING_WARNING),
            );
        }
        if let Some(total) = &total {
            root = root.child(match total {
                Ok(total) => div().child(format!("Итого: {total}")),
                Err(error) => div().text_color(danger).child(error.clone()),
            });
        }
        if let Some(error) = &self.error {
            root = root.child(div().text_color(danger).child(error.clone()));
        }
        if let Some(saved) = &self.saved {
            root = root.child(div().text_color(muted).child(saved.clone()));
        }
        root.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        can_save, optional_id_by_label, selected_kind, CARD_FIELD_LABEL, NO_CARD_LABEL,
        NO_PLACE_LABEL,
    };
    use finance_core::{parse_amount_expression, Currency, TransactionKind};

    #[test]
    fn card_field_label_is_shared_for_both_operations() {
        assert_eq!(CARD_FIELD_LABEL, "Зачислить на карту");
    }

    #[test]
    fn only_one_operation_kind_can_be_selected() {
        assert_eq!(selected_kind(&[0]), Some(TransactionKind::Income));
        assert_eq!(selected_kind(&[1]), Some(TransactionKind::Expense));
        assert_eq!(selected_kind(&[]), None);
        assert_eq!(selected_kind(&[0, 1]), None);
    }

    #[test]
    fn optional_id_by_label_matches_visible_select_items() {
        let labels = [NO_CARD_LABEL, "Карта #7"].map(str::to_owned);
        let ids: Vec<Option<i64>> = vec![None, Some(7)];

        assert_eq!(optional_id_by_label(&ids, &labels, NO_CARD_LABEL), None);
        assert_eq!(optional_id_by_label(&ids, &labels, "Карта #7"), Some(7));
        assert_eq!(optional_id_by_label(&ids, &labels, "Нет такого"), None);
        assert_eq!(
            optional_id_by_label::<i64>(&[None], &[NO_PLACE_LABEL.to_owned()], NO_PLACE_LABEL),
            None
        );
    }

    #[test]
    fn save_is_blocked_without_a_category_or_currency() {
        assert!(can_save(Some(1), Some(Currency::Rub), true));
        assert!(!can_save(None, Some(Currency::Rub), true));
        assert!(!can_save(Some(1), None, true));
        assert!(!can_save(Some(1), Some(Currency::Rub), false));
    }

    #[test]
    fn amount_expression_uses_the_shared_parser() {
        let amount = parse_amount_expression("100 + 20 - 5 * 2 / 4").expect("valid expression");
        assert_eq!(amount.to_string(), "117.50");
        assert!(parse_amount_expression("не число").is_err());
        assert!(parse_amount_expression("").is_err());
    }
}
