# Page Actions and Filters

This document records the action-button and filter patterns on the existing Income, Expenses, Savings, Payment Cards, and Analysis pages. It also recommends a consistent pattern for future pages. The recommendations below do not describe behavior already implemented everywhere.

## Header Actions

On Income, Expenses, and Savings, the page title is on the left side of the header row. The actions are aligned to the right in this order:

1. `+ Добавить`
2. `↻ Обновить`

Both buttons are 140 × 40 logical pixels. Add opens a form for a new record; fields may be prefilled with page defaults. Refresh reloads the list using the filter values currently in the controls and does not clear them.

There is a current prerequisite difference: Savings disables Add when there are no banks and explains that a bank must first be added in Directories. Income and Expenses currently keep Add enabled even if no active categories exist; they show a notice to add a category instead. Future pages should make missing prerequisites clear and should not allow an action that cannot create a valid record.

On Savings, refresh also reloads the bank list and returns the results table to its first page. Income and Expenses reload the transaction list with the current filters.

## Current Filter Behavior

### Income and Expenses

- Filters appear above the table and wrap onto additional lines when they do not fit; horizontal scrolling is not used.
- Filter groups and wrapped rows have a small, consistent gap for visual separation.
- `Применить` and `Сбросить фильтры` remain aligned to the right of the final filter row, moving to the bottom-right row when needed.
- Search text and date edits do not reload the list on their own. Use `Применить` to apply them.
- Selecting a category or currency immediately reloads the list, unlike editing search or date fields. The currency options are `Все`, `RUB`, and `USD`.
- Each populated filter, including currency, has an `×` control. Clearing a filter immediately reloads the list using the other values currently in the controls, including values that were edited but not yet applied.
- `Сбросить фильтры` clears all filters and reloads the list. It is disabled when no filters are active.
- Date boundaries are inclusive. Invalid dates or a start date later than the end date are reported as errors; the existing results are not replaced with an empty list.
- Applying filters updates the table and its totals. When no transactions match, the current page shows no data rows but has no dedicated filtered-empty-state message; future pages should provide one.

### Income and Expenses Balance

- The table footer shows the transaction count and total for the configured primary currency only. It does not convert other currencies.
- The bottom of both pages shows one `Доходы за период − Расходы за период = Сальдо за период` formula in the configured primary currency only. If that currency is not supported for transactions (currently only RUB and USD are supported), or the primary currency cannot be determined, the amount is hidden and the footer explains why.
- The balance includes every income and expense within the last successfully applied inclusive date range. Category, currency, and search filters affect the table, but do not affect this balance.
- When both date fields are empty, the balance covers all transactions in the database. Editing a date does not change the displayed result until a refresh applies the date values; the footer identifies the period used for its calculation.
- Currency totals are not converted or combined. If loading or calculating the balance fails, the old balance is hidden instead of being presented as current.
- This is the net of recorded income and expense operations, not a confirmed amount of cash on hand: opening balances and transfers are not represented by the current transaction model.

### Analysis

- Filters appear above the report and wrap onto additional lines when they do not fit; horizontal scrolling is not used.
- Filter groups and wrapped rows have a small, consistent gap for visual separation.
- `Применить` and `Сбросить фильтры` remain aligned to the right of the final filter row, moving to the bottom-right row when needed.
- Date and category edits remain pending until `Применить` is clicked. The report continues to reflect the last applied filter set.
- Each of the four filters has an `×` control. Clearing one immediately applies the other values currently in the controls, including edits that were not yet applied.
- `Сбросить фильтры` clears all filters and immediately reloads the report without conditions; it is disabled when no filters are active.
- Invalid dates or a start date later than the end date are reported without replacing the displayed report with an empty result.
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

## Recommended Pattern for Future Pages

Use the following pattern unless a page has a documented reason to differ:

1. Keep the title at the left of the header row and place `+ Добавить` followed by `↻ Обновить` at the right. Match the existing 140 × 40 logical-pixel button size.
2. Place filters above the table in a wrapping layout with a small, consistent gap between groups and rows. Keep each label and its control together, and align apply/reset actions to the right of the last row (bottom-right when the filters wrap).
3. Editing filter controls should only change their pending values; apply the full set of conditions with `Применить`.
4. Put an `×` next to each populated filter. It clears only that filter and immediately applies the remaining values. Disable it when its filter is empty.
5. Put `Сбросить фильтры` at the end of the row. It clears every filter and reloads the unfiltered list; disable it when no filters are active.
6. Combine filters with logical AND. Empty fields and `Все` mean that the corresponding condition is not applied. Update the table and its related totals together.
7. Treat `↻ Обновить` as a reload, not a reset: keep current filter values. For paginated pages, return to the first page after reloading.
8. Show a clear empty state when no records match. Validate filter input before replacing displayed results; report invalid dates or an inverted date range without silently presenting an empty result.

The recommended behavior intentionally standardizes category selection on Income and Expenses: unlike the current pages, changing any filter would wait until `Применить`, while clearing a single filter or resetting all filters would still reload immediately. Future pages can follow this pattern unless their requirements call for a documented exception.

## Form Layout

All create and edit forms, including inline forms and dialogs, must follow the shared spacing, field-layout, validation, and action-button rules in [`form-guidelines.md`](form-guidelines.md). That document defines the required button labels (`Сохранить` and `Отмена`) and their bottom-right placement; this page describes page-level actions and filters, not proof that existing forms already match the form standard.