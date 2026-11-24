//! Выделяемый текст.
//!
//! Обычный `div().child("…")` в GPUI не участвует в оконном выделении: текст
//! рисуется, но не регистрируется в слое выделения. Для текста, который
//! пользователь должен мочь протянуть мышью и скопировать (`Ctrl+C` / `Cmd+C`),
//! используется [`gpui_kit::base::SelectableText`]. Он регистрирует фрагмент в
//! `TextSelectionLayer`, а слой отрисовывает корень окна, который создаёт
//! `gpui_kit::open_window`; копирование по `Ctrl+C` обрабатывает корень.
//!
//! Текст внутри кнопок и селектов остаётся обычным. Текстовые ячейки таблиц
//! используют [`selectable_table_cell`]: он даёт каждой ячейке стабильный `ElementId`,
//! поэтому выделение не «прыгает» после сортировки, фильтрации и виртуальной
//! прокрутки. Кнопки в колонке действий остаются невыделяемыми.

use gpui_kit::base::SelectableText;
use gpui_kit::*;

/// Выделяемый текст, наследующий шрифт и цвет родителя.
pub(crate) fn selectable_text(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
) -> SelectableText {
    SelectableText::new(id, text)
}

/// Выделяемый текст сообщения или пояснения с явным цветом.
///
/// Обёртка возвращает `Div`, потому что сам `SelectableText` не реализует
/// `Styled`: к результату можно применять обычные текстовые стили
/// (`text_xs`, `text_sm`, …), а цвет задаётся здесь.
pub(crate) fn selectable_message(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    color: Hsla,
) -> Div {
    div().text_color(color).child(selectable_text(id, text))
}

fn table_cell_element_id(
    table: &'static str,
    row_id: impl std::fmt::Display,
    column: usize,
) -> String {
    format!("{table}-cell-{row_id}-{column}")
}

/// Выделяемый текст ячейки таблицы.
///
/// `row_id` должен быть стабильным идентификатором записи из БД, а не номером
/// видимой строки: таблицы виртуализированы и сортируются, поэтому индекс строки
/// меняется при прокрутке и изменении фильтров.
pub(crate) fn selectable_table_cell(
    table: &'static str,
    row_id: impl std::fmt::Display,
    column: usize,
    text: impl Into<SharedString>,
) -> SelectableText {
    selectable_text(table_cell_element_id(table, row_id, column), text)
}

#[cfg(test)]
mod tests {
    use super::table_cell_element_id;

    #[test]
    fn table_cell_ids_use_stable_row_identity_and_column() {
        assert_eq!(
            table_cell_element_id("transactions", 42_i64, 7),
            "transactions-cell-42-7"
        );
        assert_ne!(
            table_cell_element_id("transactions", 42_i64, 7),
            table_cell_element_id("transactions", 42_i64, 8)
        );
        assert_ne!(
            table_cell_element_id("transactions", 42_i64, 7),
            table_cell_element_id("transfers", 42_i64, 7)
        );
    }
}
