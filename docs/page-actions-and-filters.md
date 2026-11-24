# Page Actions and Filters

This document records the action-button and filter patterns on the existing Income, Expenses, Savings, Payment Cards, Analysis, and Directories pages. It also defines a shared header pattern for pages and sections; the standard is a recommendation and does not imply that every page already implements it.

## Header Actions

All page-level add buttons use a Plus icon instead of a textual plus and retain the icon's normal button foreground color. Add opens a form for a new record; fields may be prefilled with page defaults.

On Income and Expenses, the page title is on the left side of the header row and `Добавить доход` / `Добавить расход` with the Plus icon is aligned to the right, followed by `Обновить` with the Replace icon. Both actions use the Large button size; Add uses the Primary variant and refresh uses Secondary. Refresh reloads the transaction list with the current filter values and does not reset them.

On Directories, the header has the page title on the left and a Large Secondary `Обновить` button (Replace icon) on the right, which reloads all directories. Below it, the category-kind switch (`Категории расходов` / `Категории доходов`, with expense categories selected by default) sits on the left and the `Банки` / `Места покупок` / `Валюты` switch on the right in the same row. The add buttons for categories, banks, places, and currencies use the Small size and the Plus icon. Category create/edit forms list possible parents in the same `Родитель > Дочка` hierarchy order as the transaction category filter, with full paths in option and selected-value tooltips; a category being edited and its descendants are not offered as parents.

The Income page keeps Add enabled even if no active categories exist; it shows a notice to add a category instead. Future pages should make missing prerequisites clear and should not allow an action that cannot create a valid record.

On Savings, refresh also reloads the bank list and results with the current filters while retaining the selected sort and saved column widths. On Income, refresh reloads the transaction list with the current filters.

On Savings, Add is disabled when there are no banks and explains that a bank must first be added in Directories.

## Start Page «Главная»

The start page has no page-level action buttons. Instead of quick-action buttons, it centers one vertical compact form titled `Добавление` that creates an income or an expense directly without switching tabs or opening a modal dialog. Its large amount input accepts the same arithmetic expressions as the transaction form (with an `Итого` preview and a division-rounding warning); a large green `Добавить` button uses the `CircleCheck` icon. A segmented `Доход / Расход` group keeps exactly one type active and highlights the active option with a neutral theme background and border; switching it reloads type-specific directories and resets category and card selection while preserving the amount. Below are full-width Small selects for currency, category, and card (`Зачислить на карту` for income, `Списать с карты` for expenses); `Место траты` appears only for expenses. Records use the current date, an empty comment, and no deposit-rate effect; the actual date is shown as a small caption near the amount. Saving is disabled with an explanation when no active category exists. Descriptions of Income, Expenses, and Directories, plus the list of all pages, are links styled as text; every link only switches tabs and never modifies data. The page shows no totals or balances; directory data for the form is loaded when the page is activated, not on every render. Creating a transfer remains available from the Transfers page.

## Current Filter Behavior

Shared compact DatePicker controls retain previous/next-month arrow navigation and also let users choose a month and edit the year separately. Changing the displayed month or year does not change the date value until a day is selected; each page still applies that value according to its filter behavior below.

### Income and Expenses

- The Income table fills the available page width and orders columns as `Дата`, `Категория`, `Сумма`, `Валюта`, `Комментарии`, and `Действия`. The Expenses table adds `Место покупки` between `Валюта` and `Комментарии`. The Comments column expands into the remaining width, while Actions stays pinned at the far right with a fixed 160 px width and centered buttons. Existing saved expense-column widths are migrated so the Comments width is preserved and the new place column receives a default width. See [`table-settings.md`](table-settings.md) for the column-width setting.
- Both tables show an outer border and internal vertical and horizontal borders for the headers and all rows. The Actions column retains the table component's divider while pinned during horizontal scrolling.
- Income and Expenses comment cells show the first 25 characters of the first line; longer or multiline comments end with an ellipsis. Clicking a non-empty comment opens a dialog with the full original text.
- Income filters appear above the table and wrap onto additional lines when they do not fit; horizontal scrolling is not used. Controls are Small, each control has a label above it, and Search has a Search icon suffix. Expenses uses the same presentation.
- The filter order is `Поиск`, `Период`, start date (`С`), end date (`По`), category, bank, payment method, currency, and (on Expenses only) `Место покупки`. The payment-method filter is labelled `Способ получения` on Income and `Способ оплаты` on Expenses.
- Filter groups and wrapped rows have a small, consistent gap for visual separation.
- Category filter options show the full hierarchical category path in a tooltip when hovered in the dropdown; the selected value also shows the full path in a tooltip. Bank options show the full bank name in a tooltip in the dropdown and on the selected value, and payment-method options likewise show their full labels in both places. Expense-place options and the selected value show the full place label in a tooltip. Long labels remain visually truncated by the Select control.
- `Применить` and `Сбросить фильтры` buttons on Income and Expenses use Small size and share a bottom row aligned to the right of the filters.
- Search text and date edits do not reload the list on their own. Use `Применить` to apply them.
- The date filters use compact DatePicker controls. The `Период` selector defaults to `Весь период`. Selecting it clears both dates; selecting year, quarter, month, week (Monday through Sunday), or day fills both controls using the current local calendar, with inclusive boundaries. Users can choose either date afterward. Use `Применить` to apply the selected range.
- Selecting a category, bank, payment method, or currency immediately reloads the list, unlike editing search or date fields. The currency options are `Все`, `RUB`, and `USD`. On Expenses, selecting a place immediately reloads the list; `Все` includes expenses with and without a place.
- The payment-method filter offers `Все`, `Не по карте`, then one option per available card. `Все` keeps both records with and without a card; `Не по карте` keeps only records whose `payment_document_id` is NULL.
- The bank and payment-method filters have a `Все` option. With no bank selected, the card options include every card; selecting a bank restricts the options to cards from that bank. Changing the bank clears the selected payment method. Card options are not restricted by card type or currency.
- Each populated filter, including currency, has an `×` control. Clearing a filter immediately reloads the list using the other values currently in the controls, including values that were edited but not yet applied.
- `Сбросить фильтры` clears all filters, returns the period selector to `Весь период`, and reloads the list. It is disabled when no filters are active.
- Date boundaries are inclusive. A start date later than the end date is reported as an error; the existing results are not replaced with an empty list. Dates are selected through the calendar rather than entered as free-form text.
- Applying filters updates the table and its totals. When no transactions match, the current page shows no data rows but has no dedicated filtered-empty-state message; future pages should provide one.
- Creating and editing an income or expense opens a modal dialog over the page. The dialog keeps its fields and validation errors while open; clicking `Отмена` asks for confirmation if values have changed, and the dialog cannot be dismissed by clicking the overlay, pressing Escape, or using a close button.
- The expense form allows choosing an optional place from the expense-place directory or `Не указано`; the value is stored with the expense and restored when editing it. Income forms do not show this field.
- The income and expense form date is selected with a DatePicker rather than entered as free-form text.

### Income and Expenses Comment Suggestions

- The transaction form offers suggestions from the distinct, non-empty comments already stored for both incomes and expenses.
- Suggestions are loaded when an add/edit form opens and do not depend on the currently applied table filters.
- While typing, up to five comments containing the entered text are shown, matched without regard to letter case. Blank input and an exact match do not show suggestions; selecting one replaces the comment field with the saved comment.

### Income and Expenses Balance

- The table footer shows the transaction count and total for the configured primary currency only. It does not convert other currencies.
- The bottom of both pages shows the applied date range on a separate line before one `Доходы − Расходы = Сальдо` formula (for example, `За период с … по …:`). With no date boundaries, the separate line says `За весь период:`. The period line and balance formula use the same prominent text size; the formula uses the configured primary currency only. If that currency is not supported for transactions (currently only RUB and USD are supported), or the primary currency cannot be determined, the amount is hidden and the footer explains why.
- The balance includes every income and expense within the last successfully applied inclusive date range. Category, bank, payment-method, currency, and search filters affect the table, but do not affect this balance.
- When both date fields are empty, the balance covers all transactions in the database and the footer shows `За весь период:` above the formula. Editing a date does not change the displayed result until a refresh applies the date values; the footer identifies the applied period once, above the balance formula.
- Currency totals are not converted or combined. If loading or calculating the balance fails, the old balance is hidden instead of being presented as current.
- This is the net of recorded income and expense operations, not a confirmed amount of cash on hand: opening balances and transfers are not represented by the current transaction model.

### Analysis

- Filters appear above the report and wrap onto additional lines when they do not fit; horizontal scrolling is not used.
- Filter groups and wrapped rows have a small, consistent gap for visual separation.
- `Применить` and `Сбросить фильтры` remain aligned to the right of the final filter row, moving to the bottom-right row when needed.
- Date and category edits remain pending until `Применить` is clicked. The report continues to reflect the last applied filter set.
- Options in the `Категория доходов` and `Категория расходов` filters show the full hierarchical category path in a tooltip in the dropdown and on the selected value.
- The date filters use compact DatePicker controls. The `Период` selector defaults to `Весь период`. Selecting it clears both dates; selecting year, quarter, month, week (Monday through Sunday), or day fills both controls using the current local calendar, with inclusive boundaries. Users can choose either date afterward. Date changes and period selection remain pending until `Применить` is clicked.
- Each of the four filters has an `×` control. Clearing one immediately applies the other values currently in the controls, including edits that were not yet applied.
- `Сбросить фильтры` clears all filters, returns the period selector to `Весь период`, and immediately reloads the report without conditions; it is disabled when no filters are active.
- A start date later than the end date is reported without replacing the displayed report with an empty result. Dates are selected through the calendar rather than entered as free-form text.
- The page refresh action reloads the categories and applies the values currently in the controls.

### Savings

- Search, bank, currency, status, term, and interest-destination filters appear above the table and wrap onto additional lines when they do not fit; horizontal scrolling is not used.
- Filter groups and wrapped rows have a small, consistent gap for visual separation.
- Bank, term, and interest-destination options show their full labels in a tooltip when hovered in the dropdown.
- `Применить` and `Сбросить фильтры` remain aligned to the right of the final filter row, moving to the bottom-right row when needed.
- Search edits do not reload the results until `Применить` is clicked or Enter is pressed. Selecting bank, currency, status, term, or interest destination applies the filter immediately.
- The Savings results use a resizable `DataTable`; column widths are persisted in the database and legacy widths are remapped when the column order changes. Columns appear as `Дата открытия` followed by `Срок`. Grouped headers are `Ставка` over `Базовая`, `Повышенная`, `Условие` and `Начисление процентов` over `Дата`, `Назначение`. Sorting applies to the complete filtered result set for `Банк`, `Название`, `Сумма`, `Базовая`, `Повышенная`, `Условие`, `Дата открытия`, and `Срок`.
- The term filter offers `Бессрочный` and `С ограниченным сроком`; the interest-destination filter offers `Капитализация` and `На карту`. These filters combine with the other selected conditions.
- Currency options are `Все`, `RUB`, and `USD`. A currency selection matches only accounts explicitly set to that currency; `Все` includes accounts with an unknown currency.
- The search input has an inline clear control; its value is applied on `Применить` or Enter. Select filters are cleared with `Сбросить фильтры`.
- `Сбросить фильтры` clears all filters, including term and interest destination, and reloads the list. It is disabled when no filters are active.
- The `↻ Обновить` header action reloads the banks and results with the current filter values without resetting the sort or column widths.
- If no records match, the page shows an empty-state message.
- Opening date and interest-accrual date are optional `YYYY-MM-DD` values in the add/edit form; clearing either field stores it as unknown, shown as `—` in the table. These are manually maintained reference dates and do not trigger interest calculations or assert that an accrual was verified by the bank.

### Payment Cards

- Bank, payment system, card type, and currency filters appear above the table and wrap onto additional lines when they do not fit; horizontal scrolling is not used.
- Filter groups and wrapped rows have a small, consistent gap for visual separation.
- `Применить` and `Сбросить фильтры` remain aligned to the right of the final filter row, moving to the bottom-right row when needed.
- Currency options are `Все`, `RUB`, and `USD`; selecting a currency and clicking `Применить` filters the full combination of selected conditions. A currency-specific selection excludes cards with unknown currency.
- The `×` control clears the currency filter and immediately reloads using the other conditions. `Сбросить фильтры` clears all four filters and reloads the full list.
- In the `Следующий платёж` group, a fixed-width unnamed column (40px) shows an XSmall button with the `Inspector` icon only if either the payment date or amount is present. The dialog edits the payment date and amount without opening the full card editor.
- `Сформировать расход` reveals the expense fields: amount, currency, active category, optional same-currency payment card (default `Без карты`), optional expense place, date, and comment. Expense date and amount initially match the payment values when the dialog opened, and are edited independently of the new payment values. Currency initially matches the card's currency, if known. Missing required values must be entered explicitly; validation errors do not modify the card or create an expense.
- Saving the optional expense and the edited next-payment data is one database transaction. A successful operation reloads the card table; a failed expense validation rolls back the next-payment changes too. Cancelling after edits asks for confirmation.

### Transfers

- The header shows the title on the left and Large `Добавить перевод` (Plus, Primary) and `Обновить` (Replace, Secondary) actions on the right. Add is disabled until at least one debit card or active savings account with an explicit currency exists; refresh reloads accounts and the journal without changing filters, sort, or saved column widths.
- Filters appear above the balance summary and the journal in a wrapping layout: `Поиск`, date range (`С`/`По`), `Тип операции`, `Источник`, `Назначение`, and `Валюта`. Editing a control only changes its pending value; the full set is applied by `Применить` or Enter in search. Each populated filter has an `×` that clears only that filter and immediately reloads. `Сбросить фильтры` is disabled when no filter is active. Conditions combine with AND; date boundaries are inclusive. Empty results show a dedicated message.
- Source and destination filters accept `Все`, external funds, cash, every card, and every savings account, including accounts that are no longer eligible for new transfers so historical records remain reachable.
- Every option in the `Тип операции`, `Источник`, and `Назначение` selects shows its full label in a tooltip.
- The balance summary keeps RUB and USD separate, shows card and savings totals individually, and omits credit-card limits and accounts with unknown currency. A footnote explains that top-up and withdrawal do not create income or expense records and that the sum of balances can therefore differ from the Analysis report.
- The add/edit form is an 800 pt modal dialog with `Дата`, `Тип операции`, `Откуда`, `Куда`, `Сумма`, `Валюта`, and `Комментарий`; dependent endpoint fields change with operation type and currency. Only Сохранить and Отмена are used, validation errors stay next to the form, and cancelling with unsaved changes asks for confirmation. Warnings explain that a top-up creates no income, that withdrawal removes money from tracked balances, and that early withdrawal from a limited-term savings account may have bank-specific conditions.
- Deletion asks for confirmation showing date, amount, and both endpoints. Table actions require a row and the journal is never reloaded on every render frame.

### Directories — Prices

- The prices filters use the same compact presentation as Income: labels above Small controls, a wrapping row with a small consistent gap, and `Применить` / `Сбросить фильтры` aligned to the right on a separate bottom row.
- Filter order is `Поиск`, `Период`, `Дата от`, `Дата до`, `Валюта`, `Место покупки`. Search has a Search icon suffix and inline clear control.
- Search is case-insensitive and matches the product name or the expense-place name. Search and date edits remain pending until `Применить` is clicked or Enter is pressed in search.
- The `Период` selector defaults to `Весь период` and uses the same calendar ranges as Income; selecting a preset fills both date controls and reloads immediately. Date boundaries are inclusive, and an inverted range is reported without replacing the displayed rows with an empty list.
- Currency and place selections reload immediately, as on Income. Empty search and `Все валюты` / `Все места` mean that the condition is not applied.
- `Сбросить фильтры` clears every filter, returns the period selector to `Весь период`, and reloads the full list. It is disabled when no filter is active.

## Shared Header Pattern for Pages and Sections

Use the same header layout on every page and section:

1. Use a full-width header row with the page or section title on the left and its available action buttons grouped on the right, vertically centered on the same row.
2. Render page titles in bold 24 logical-pixel text. Use a consistent, visually subordinate heading size for sections while keeping the same alignment and action placement.
3. Show only actions that apply to that page or section. When available, place `Добавить` before `↻ Обновить` and use a Plus icon instead of a textual plus. Do not add an unavailable action or leave a placeholder button. If there are no actions, keep the title in its usual left position.
4. Keep action buttons consistent in size within their action group. The Income add action uses Large, and add actions on Directories use the default Medium size. If the available width is insufficient, move the action group to a second row aligned to the right rather than allowing it to overlap the title.

## Recommended Filter Pattern for Future Pages

Use the following filter pattern unless a page has a documented reason to differ:

1. Place filters above the table in a wrapping layout with a small, consistent gap between groups and rows. Keep each label and its control together, and align apply/reset actions to the right of the last row (bottom-right when the filters wrap).
2. Editing filter controls should only change their pending values; apply the full set of conditions with `Применить`.
3. Put an `×` next to each populated filter. It clears only that filter and immediately applies the remaining values. Disable it when its filter is empty.
4. Put `Сбросить фильтры` at the end of the row. It clears every filter and reloads the unfiltered list; disable it when no filters are active.
5. Combine filters with logical AND. Empty fields and `Все` mean that the corresponding condition is not applied. Update the table and its related totals together.
6. Treat `↻ Обновить` as a reload, not a reset: keep current filter values. For paginated pages, return to the first page after reloading.
7. Show a clear empty state when no records match. Validate filter input before replacing displayed results; report invalid dates or an inverted date range without silently presenting an empty result.

The recommended behavior intentionally standardizes category selection on Income and Expenses: unlike the current pages, changing any filter would wait until `Применить`, while clearing a single filter or resetting all filters would still reload immediately. Future pages can follow this pattern unless their requirements call for a documented exception.

## Form Layout

All create and edit forms, including inline forms and dialogs, must follow the shared spacing, field-layout, validation, and action-button rules in [`form-guidelines.md`](form-guidelines.md). That document defines the required button labels (`Сохранить` and `Отмена`) and their bottom-right placement; this page describes page-level actions and filters, not proof that existing forms already match the form standard.