# Page Actions and Filters

This document records the action-button and filter patterns on the existing Income, Expenses, and Savings pages. It also recommends a consistent pattern for future pages. The recommendations below do not describe behavior already implemented everywhere.

## Header Actions

On Income, Expenses, and Savings, the page title is on the left side of the header row. The actions are aligned to the right in this order:

1. `＋ Добавить`
2. `↻ Обновить`

Both buttons are 140 × 40 logical pixels. Add opens a form for a new record; fields may be prefilled with page defaults. Refresh reloads the list using the filter values currently in the controls and does not clear them.

There is a current prerequisite difference: Savings disables Add when there are no banks and explains that a bank must first be added in Directories. Income and Expenses currently keep Add enabled even if no active categories exist; they show a notice to add a category instead. Future pages should make missing prerequisites clear and should not allow an action that cannot create a valid record.

On Savings, refresh also reloads the bank list and returns the results table to its first page. Income and Expenses reload the transaction list with the current filters.

## Current Filter Behavior

### Income and Expenses

- Filters appear in a horizontal row above the table; the row can scroll horizontally when it does not fit.
- Search text and date edits do not reload the list on their own. Use `Применить` to apply them.
- Selecting a category immediately reloads the list, unlike editing search or date fields.
- Each populated filter has an `×` control. Clearing a filter immediately reloads the list using the other values currently in the controls, including values that were edited but not yet applied.
- `Сбросить фильтры` clears all filters and reloads the list. It is disabled when no filters are active.
- Date boundaries are inclusive. Invalid dates or a start date later than the end date are reported as errors; the existing results are not replaced with an empty list.
- Applying filters updates the table and its totals. When no transactions match, the current page shows no data rows but has no dedicated filtered-empty-state message; future pages should provide one.

### Savings

- Search, bank, and status filters appear in a horizontally scrollable row above the table.
- Editing search, bank, or status does not reload the results until `Применить` is clicked.
- Each active filter has an `×` control. Clearing one immediately reloads the results using the other values currently in the controls.
- `Сбросить фильтры` clears all filters and reloads the list. It is disabled when no filters are active.
- The `↻ Обновить` header action reloads the banks and results with the current filter values, then returns the table to its first page.
- If no records match, the page shows an empty-state message.

## Recommended Pattern for Future Pages

Use the following pattern unless a page has a documented reason to differ:

1. Keep the title at the left of the header row and place `＋ Добавить` followed by `↻ Обновить` at the right. Match the existing 140 × 40 logical-pixel button size.
2. Place filters in one row above the table. Allow horizontal scrolling if they do not fit.
3. Editing filter controls should only change their pending values; apply the full set of conditions with `Применить`.
4. Put an `×` next to each populated filter. It clears only that filter and immediately applies the remaining values. Disable it when its filter is empty.
5. Put `Сбросить фильтры` at the end of the row. It clears every filter and reloads the unfiltered list; disable it when no filters are active.
6. Combine filters with logical AND. Empty fields and `Все` mean that the corresponding condition is not applied. Update the table and its related totals together.
7. Treat `↻ Обновить` as a reload, not a reset: keep current filter values. For paginated pages, return to the first page after reloading.
8. Show a clear empty state when no records match. Validate filter input before replacing displayed results; report invalid dates or an inverted date range without silently presenting an empty result.

The recommended behavior intentionally standardizes category selection on Income and Expenses: unlike the current pages, changing any filter would wait until `Применить`, while clearing a single filter or resetting all filters would still reload immediately. Future pages can follow this pattern unless their requirements call for a documented exception.