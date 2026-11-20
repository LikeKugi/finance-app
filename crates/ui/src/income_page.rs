use eframe::egui;

use super::{transactions_page, FinanceApp, TransactionKind};

pub(super) fn render(app: &mut FinanceApp, ui: &mut egui::Ui) {
    transactions_page::render(app, ui, TransactionKind::Income);
}
