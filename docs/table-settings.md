# Table Settings

## Income and Expenses

- Both tables have an outer border and internal vertical and horizontal borders for every column and row, including the headers.
- The pinned `Действия` column is 160 logical pixels wide on both pages.
- The `Изменить` and `Удалить` buttons are centered together in the column, leaving equal horizontal space on each side and keeping both buttons on one line.
- This width overrides any previously saved width for the Actions column; saved widths for other columns are preserved.

## Savings

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
