use chrono::NaiveDate;
use eframe::egui;
use finance_core::{Currency, FinancialTransaction, TransactionBalance};
use rust_decimal::Decimal;

use super::{AnalysisPage, ModuleTab};

pub(super) const TABLE_ROW_HEIGHT: f32 = 48.0;
const FILTER_ACTIONS_MIN_WIDTH: f32 = 250.0;
const FILTER_ITEM_SPACING: f32 = 8.0;

pub(super) fn filter_layout(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(FILTER_ITEM_SPACING, FILTER_ITEM_SPACING);
        ui.horizontal_wrapped(add_contents);
    });
}

pub(super) fn filter_group(
    ui: &mut egui::Ui,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        add_contents,
    );
}

pub(super) fn render_filter_actions(ui: &mut egui::Ui, has_active_filters: bool) -> (bool, bool) {
    let available_width = ui.available_size_before_wrap().x;
    let width = if available_width < FILTER_ACTIONS_MIN_WIDTH {
        ui.max_rect().width()
    } else {
        available_width
    };
    ui.allocate_ui_with_layout(
        egui::vec2(width, ui.spacing().interact_size.y),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            let reset = ui
                .add_enabled(has_active_filters, egui::Button::new("Сбросить фильтры"))
                .clicked();
            let apply = ui.button("Применить").clicked();
            (apply, reset)
        },
    )
    .inner
}

pub(super) fn render_analysis_footer(
    context: &egui::Context,
    active_tab: ModuleTab,
    analysis_page: &AnalysisPage,
) {
    if active_tab == ModuleTab::Analysis {
        egui::TopBottomPanel::bottom("analysis_footer").show(context, |ui| {
            ui.separator();
            analysis_page.render_footer(ui);
        });
    }
}

pub(super) fn transaction_balance_period_label(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> String {
    match (from, to) {
        (None, None) => "за весь период".to_owned(),
        (Some(from), Some(to)) => format!(
            "за период с {} по {}",
            from.format("%d.%m.%Y"),
            to.format("%d.%m.%Y")
        ),
        (Some(from), None) => format!("за период с {}", from.format("%d.%m.%Y")),
        (None, Some(to)) => format!("за период по {}", to.format("%d.%m.%Y")),
    }
}

pub(super) fn render_transaction_balance_footer(
    ui: &mut egui::Ui,
    balances: Option<&[TransactionBalance; 2]>,
    period: Option<(Option<NaiveDate>, Option<NaiveDate>)>,
    primary_currency: Option<&str>,
) {
    match (balances, period) {
        (Some(balances), Some((from, to))) => {
            let Some(primary_currency) = primary_currency else {
                ui.colored_label(
                    egui::Color32::RED,
                    "Сальдо не отображается: не удалось определить основную валюту.",
                );
                return;
            };
            let Some(balance) = balances
                .iter()
                .find(|balance| balance.currency.code() == primary_currency)
            else {
                ui.colored_label(
                    egui::Color32::RED,
                    format!(
                        "Сальдо не отображается: основная валюта {primary_currency} не поддерживается для операций (доступны RUB и USD)."
                    ),
                );
                return;
            };
            let period_label = transaction_balance_period_label(from, to);
            ui.label(
                egui::RichText::new(format!(
                    "Доходы {period_label}: {} {} − Расходы {period_label}: {} {} = Сальдо {period_label}: {} {}",
                    balance.income,
                    balance.currency.code(),
                    balance.expenses,
                    balance.currency.code(),
                    balance.balance,
                    balance.currency.code(),
                ))
                .size(egui::TextStyle::Body.resolve(ui.style()).size + 4.0)
                .strong(),
            );
            ui.weak(
                "Сальдо операций не включает начальные остатки и переводы и не подтверждает сумму денег на руках.",
            );
        }
        _ => {
            ui.colored_label(
                egui::Color32::RED,
                "Сальдо не рассчитано: обновите список после устранения ошибки.",
            );
        }
    }
}

pub(super) fn render_transaction_list_total(
    ui: &mut egui::Ui,
    transactions: &[FinancialTransaction],
    primary_currency: Option<&str>,
) -> bool {
    let Some(primary_currency) = primary_currency else {
        ui.label(format!(
            "Записей: {}   Итого недоступно: основная валюта не определена",
            transactions.len()
        ));
        return false;
    };
    let Some(currency) = Currency::ALL
        .into_iter()
        .find(|currency| currency.code() == primary_currency)
    else {
        ui.label(format!(
            "Записей: {}   Итого недоступно: основная валюта {primary_currency} не поддерживается для операций",
            transactions.len()
        ));
        return false;
    };
    let Some(total) = transactions
        .iter()
        .filter(|transaction| transaction.currency == currency)
        .try_fold(Decimal::ZERO, |total, transaction| {
            total.checked_add(transaction.amount)
        })
    else {
        ui.colored_label(
            egui::Color32::RED,
            "Не удалось рассчитать итог: сумма слишком велика",
        );
        return true;
    };

    ui.label(format!(
        "Записей: {}   Итого: {total} {primary_currency}",
        transactions.len()
    ));
    false
}
