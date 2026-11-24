//! Выделяемый текст.
//!
//! Обычный `div().child("…")` в GPUI не участвует в оконном выделении: текст
//! рисуется, но не регистрируется в слое выделения. Для текста, который
//! пользователь должен мочь протянуть мышью и скопировать (`Ctrl+C` / `Cmd+C`),
//! используется [`gpui_kit::base::SelectableText`]. Он регистрирует фрагмент в
//! `TextSelectionLayer`, а слой отрисовывает корень окна, который создаёт
//! `gpui_kit::open_window`; копирование по `Ctrl+C` обрабатывает корень.
//!
//! Текст внутри кнопок, селектов и ячеек таблиц остаётся обычным: таблица
//! перехватывает клик по строке, а строки виртуализированы, поэтому выделение
//! там решается отдельно (полный комментарий доступен в диалоге).

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
