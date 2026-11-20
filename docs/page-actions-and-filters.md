# Page Actions and Filters

This document records the action-button and filter patterns on the existing Income, Expenses, Savings, Payment Cards, and Analysis pages. It also defines a shared header pattern for pages and sections; the standard is a recommendation and does not imply that every page already implements it.

## Header Actions

On Income, the page title is on the left side of the header row and `+ Добавить доход` is aligned to the right. This header has no refresh action. Add opens a form for a new record; fields may be prefilled with page defaults.

The Income page keeps Add enabled even if no active categories exist; it shows a notice to add a category instead. Future pages should make missing prerequisites clear and should not allow an action that cannot create a valid record.

On Savings, refresh also reloads the bank list and returns the results table to its first page. Income reloads the transaction list with the current filters when filters are applied.

On Savings, Add is disabled when there are no banks and explains that a bank must first be added in Directories.

## Current Filter Behavior

Shared compact DatePicker controls retain previous/next-month arrow navigation and also let users choose a month and edit the year separately. Changing the displayed month or year does not change the date value until a day is selected; each page still applies that value according to its filter behavior below.

### Income and Expenses

- The Income table fills the available page width and orders columns as `Дата`, `Категория`, `Сумма`, `Валюта`, `Комментарии`, and `Действия`. The Comments column expands into the remaining width, while Actions stays at the far right; Expenses retains its existing column order.
- Income comment cells show the first 25 characters of the first line; longer or multiline comments end with an ellipsis. Clicking a non-empty comment opens a dialog with the full original text.
- Filters appear above the table and wrap onto additional lines when they do not fit; horizontal scrolling is not used.
- The filter order is `Поиск`, `Период`, start date (`С`), end date (`По`), category, and currency.
- Filter groups and wrapped rows have a small, consistent gap for visual separation.
- `Применить` and `Сбросить фильтры` remain aligned to the right of the final filter row, moving to the bottom-right row when needed.
- Search text and date edits do not reload the list on their own. Use `Применить` to apply them.
- The date filters use compact DatePicker controls. The `Период` selector defaults to `Весь период`. Selecting it clears both dates; selecting year, quarter, month, week (Monday through Sunday), or day fills both controls using the current local calendar, with inclusive boundaries. Users can choose either date afterward. Use `Применить` to apply the selected range.
- Selecting a category or currency immediately reloads the list, unlike editing search or date fields. The currency options are `Все`, `RUB`, and `USD`.
- Each populated filter, including currency, has an `×` control. Clearing a filter immediately reloads the list using the other values currently in the controls, including values that were edited but not yet applied.
- `Сбросить фильтры` clears all filters, returns the period selector to `Весь период`, and reloads the list. It is disabled when no filters are active.
- Date boundaries are inclusive. A start date later than the end date is reported as an error; the existing results are not replaced with an empty list. Dates are selected through the calendar rather than entered as free-form text.
- Applying filters updates the table and its totals. When no transactions match, the current page shows no data rows but has no dedicated filtered-empty-state message; future pages should provide one.

### Income and Expenses Comment Suggestions

- The transaction form offers suggestions from the distinct, non-empty comments already stored for both incomes and expenses.
- Suggestions are loaded when an add/edit form opens and do not depend on the currently applied table filters.
- While typing, up to five comments containing the entered text are shown, matched without regard to letter case. Blank input and an exact match do not show suggestions; selecting one replaces the comment field with the saved comment.

### Income and Expenses Balance

- The table footer shows the transaction count and total for the configured primary currency only. It does not convert other currencies.
- The bottom of both pages shows the applied date range on a separate line before one `Доходы − Расходы = Сальдо` formula (for example, `За период с … по …:`). With no date boundaries, the separate line says `За весь период:`. The period line and balance formula use the same prominent text size; the formula uses the configured primary currency only. If that currency is not supported for transactions (currently only RUB and USD are supported), or the primary currency cannot be determined, the amount is hidden and the footer explains why.
- The balance includes every income and expense within the last successfully applied inclusive date range. Category, currency, and search filters affect the table, but do not affect this balance.
- When both date fields are empty, the balance covers all transactions in the database and the footer shows `За весь период:` above the formula. Editing a date does not change the displayed result until a refresh applies the date values; the footer identifies the applied period once, above the balance formula.
- Currency totals are not converted or combined. If loading or calculating the balance fails, the old balance is hidden instead of being presented as current.
- This is the net of recorded income and expense operations, not a confirmed amount of cash on hand: opening balances and transfers are not represented by the current transaction model.

### Analysis

- Filters appear above the report and wrap onto additional lines when they do not fit; horizontal scrolling is not used.
- Filter groups and wrapped rows have a small, consistent gap for visual separation.
- `Применить` and `Сбросить фильтры` remain aligned to the right of the final filter row, moving to the bottom-right row when needed.
- Date and category edits remain pending until `Применить` is clicked. The report continues to reflect the last applied filter set.
- The date filters use compact DatePicker controls. The `Период` selector defaults to `Весь период`. Selecting it clears both dates; selecting year, quarter, month, week (Monday through Sunday), or day fills both controls using the current local calendar, with inclusive boundaries. Users can choose either date afterward. Date changes and period selection remain pending until `Применить` is clicked.
- Each of the four filters has an `×` control. Clearing one immediately applies the other values currently in the controls, including edits that were not yet applied.
- `Сбросить фильтры` clears all filters, returns the period selector to `Весь период`, and immediately reloads the report without conditions; it is disabled when no filters are active.
- A start date later than the end date is reported without replacing the displayed report with an empty result. Dates are selected through the calendar rather than entered as free-form text.
- The page refresh action reloads the categories and applies the values currently in the controls.

### Savings

- Search, bank, currency, and status filters appear above the table and wrap onto additional lines when they do not fit; horizontal scrolling is not used.
- Filter groups and wrapped rows have a small, consistent gap for visual separation.
- `Применить` and `Сбросить фильтры` remain aligned to the right of the final filter row, moving to the bottom-right row when needed.
- Editing search, bank, currency, or status does not reload the results until `Применить` is clicked.
- Currency options are `Все`, `RUB`, and `USD`. A currency selection matches only accounts explicitly set to that currency; `Все` includes accounts with an unknown currency.
- Each active filter has an `×` control. Clearing one immediately reloads the results using the other values currently in the controls.
- `Сбросить фильтры` clears all filters and reloads the list. It is disabled when no filters are active.
- The `↻ Обновить` header action reloads the banks and results with the current filter values, then returns the table to its first page.
- If no records match, the page shows an empty-state message.
- Opening date and interest-accrual date are optional `YYYY-MM-DD` values in the add/edit form; clearing either field stores it as unknown, shown as `—` in the table. These are manually maintained reference dates and do not trigger interest calculations or assert that an accrual was verified by the bank.

### Payment Cards

- Bank, payment system, card type, and currency filters appear above the table and wrap onto additional lines when they do not fit; horizontal scrolling is not used.
- Filter groups and wrapped rows have a small, consistent gap for visual separation.
- `Применить` and `Сбросить фильтры` remain aligned to the right of the final filter row, moving to the bottom-right row when needed.
- Currency options are `Все`, `RUB`, and `USD`; selecting a currency and clicking `Применить` filters the full combination of selected conditions. A currency-specific selection excludes cards with unknown currency.
- The `×` control clears the currency filter and immediately reloads using the other conditions. `Сбросить фильтры` clears all four filters and reloads the full list.

## Shared Header Pattern for Pages and Sections

Use the same header layout on every page and section:

1. Use a full-width header row with the page or section title on the left and its available action buttons grouped on the right, vertically centered on the same row.
2. Render page titles in bold 24 logical-pixel text. Use a consistent, visually subordinate heading size for sections while keeping the same alignment and action placement.
3. Show only actions that apply to that page or section. When available, place `+ Добавить` before `↻ Обновить`; do not add an unavailable action or leave a placeholder button. If there are no actions, keep the title in its usual left position.
4. Keep action buttons consistent in size; the recommended size is 140 × 40 logical pixels. If the available width is insufficient, move the action group to a second row aligned to the right rather than allowing it to overlap the title.

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