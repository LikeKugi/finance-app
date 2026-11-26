# Table Settings

## Text selection in numeric cells

- Right-aligned selectable cells use a flex container with `justify_end` instead of `text_right()` on the text itself. GPUI applies `text_align` when painting glyphs but not in `TextLayout::position_for_index()`, which the selection layer uses for highlight quads. With `text_right()` the digits were painted at the right edge while the highlight stayed at the left edge of the cell. Do not reintroduce `text_right()` on `SelectableText` cells.

## Income and Expenses

- Both tables have an outer border and internal vertical and horizontal borders for every column and row, including the headers.
- The pinned `Действия` column is 160 logical pixels wide on both pages.
- The Expenses table has no `Регулярные` column; its columns are `Дата`, `Категория`, `Сумма`, `Валюта`, `Место покупки`, `Комментарии`, and `Действия`.
- Existing saved expense widths are migrated in memory: six legacy values gain `Место покупки`, eight earlier values drop the removed `Регулярные` width, and all remaining saved widths keep their order. The fixed Actions width is applied after migration.
- The `Изменить` and `Удалить` buttons are centered together in the column, leaving equal horizontal space on each side and keeping both buttons on one line.
- This width overrides any previously saved width for the Actions column; saved widths for other columns are preserved.

## Savings

- The columns are `Название`, `Банк`, `Сумма`, `Валюта`, `Базовая`, `Повышенная`, `Условие`, `Дата открытия`, `Срок`, `Дата`, `Назначение`, and `Действия`.
- The grouped header `Ставка` spans `Базовая`, `Повышенная`, and `Условие`; the grouped header `Начисление процентов` spans `Дата` and `Назначение`. Other columns keep empty group-header cells so their positions and the pinned actions column are unchanged.
- Saved widths use the `table_columns.savings.v3` setting. Widths stored under `table_columns.savings.v2` are reordered to follow their columns after `Название` and `Банк` were swapped; widths under the oldest `table_columns.savings` key are remapped for that swap and for the earlier `Дата открытия`/`Срок` swap. Invalid or missing values fall back to defaults.
- The table has an outer border and internal vertical and horizontal borders for every column and row, including the headers.
- The pinned `Действия` column is 160 logical pixels wide.
- The `Изменить` and `Удалить` buttons are centered together in the column, leaving equal horizontal space on each side and keeping both buttons on one line.
- This width overrides any previously saved width for the Actions column, including legacy settings; saved widths for other columns are preserved.

## Payment Cards

- The table columns are `Название`, `Банк`, `Система`, `Тип`, `Сумма`, `Валюта`, `Льгота, дни`, then the grouped header `Следующий платёж` over `Дата`, `Размер`, and an unnamed edit-action column, followed by `Условие ставки`, `Минимум трат`, `Связанные накопления`, and `Действия`.
- The unnamed next-payment action column is fixed at 40 logical pixels and cannot be resized. Its XSmall `Inspector` icon button appears only when a card has a next-payment date or amount.
- Existing 13-column width settings gain the fixed-width action column without changing the saved widths of other columns.
- The table has an outer border and internal vertical and horizontal borders for every column and row, including the headers.
- The pinned `Действия` column is 160 logical pixels wide.
- The `Изменить` and `Удалить` buttons are centered together in the column, leaving equal horizontal space on each side and keeping both buttons on one line.
- This width overrides any previously saved width for the Actions column; saved widths for other columns are preserved.
- Saved widths use the `table_columns.payment_documents.v2` setting. Widths stored under the previous `table_columns.payment_documents` key are reordered to follow their columns after the date and grace-period columns were swapped; invalid or missing values fall back to defaults.
## Transfers

- The journal columns are `Дата`, `Операция`, `Откуда`, `Куда`, `Сумма`, `Валюта`, `Комментарий`, and `Действия`.
- Saved widths use the `table_columns.transfers.v1` setting; invalid or missing values fall back to defaults.
- The table has an outer border and internal vertical and horizontal borders for every column and row, including the headers.
- The pinned `Действия` column is 160 logical pixels wide. The saved width for that column is ignored and the other widths are preserved.

## Directories

- The `Категории`, `Банки`, `Валюты`, `Места трат` and `Цены` tables use `DataTable` with built-in virtual scrolling.
- Each table is sized for exactly 10 data rows plus the header row (352 logical pixels with the default 32-pixel row height). When a directory has more rows, the table body scrolls without rendering the whole list.
- The `Действия` column is pinned to the right edge and is not selectable or resizable.
- Category rows keep their tree indentation and inactive-state marker; currency and place cells truncate long values and expose the full rate or comment text through a tooltip or the existing comment dialog.
- The `Банки`, `Валюты` and `Места покупок` tables share the same full-width slot in the right half of the page. The preferred `Валюты` column widths are `Валюта` (80 px), `Курс` (160 px), `Основная` (240 px) and `Действия` (160 px). Together, `Валюта`, `Курс` and `Основная` take 480 px — exactly the width of the `Название` column in the `Банки` table — so both tables have the same total width of 640 px. The `Основная` column still fits the full `Назначить основной` label without truncating it.
- The `Валюта` cell shows only the code for `RUB` and `USD`; full names are omitted to keep the narrow column readable. Other currencies keep the `код название` format.
- The primary-currency button is labelled `Назначить основной`; the exchange-rate button is labelled `Изменить`. Their tooltips explain the action or why it is unavailable.
