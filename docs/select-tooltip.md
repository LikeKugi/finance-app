# Тултипы для опций `Select`

Этот документ описывает приём, который уже используется на страницах «Доходы» и «Расходы» в фильтрах «Категория», «Банк» и «Способ получения» (`crates/ui/src/gpui/transactions.rs`) и в форме категории на странице «Справочники» (`crates/ui/src/gpui/directories.rs`).

Тултипы нужны, когда подпись варианта длиннее отведённой под селект ширины. Для категорий подпись содержит полный путь (`Транспорт > Разное > История`), поэтому в выпадающем списке и в выбранном значении показывается полный текст, а элемент остаётся компактным.

## Принцип

У `Select` из `gpui-kit` 0.10 нет отдельного метода «добавить tooltip к опции». Стандартный элемент просто выводит `title()`. Поэтому для тултипов создаётся собственный тип элемента, реализующий `SelectItem` (это алиас `SearchableListItem`), а метод `render()` переопределяется: в него кладётся `div` с уникальным `id`, на который и навешивается `.tooltip(...)`.

Тултип выбранного значения — отдельная задача: `render()` работает только для строк открытого списка. Для самого `Select` тултип вешается на обёртку с `div().id(...)` вокруг `Select::new(...)`.

## 1. Свой тип элемента

```rust
use gpui_kit::component::select::{SelectEvent, SelectItem, SelectState};
use gpui_kit::component::tooltip::Tooltip as ComponentTooltip;
use gpui_kit::prelude::{FluentBuilder as _, StatefulInteractiveElement};
use gpui_kit::*;

#[derive(Clone)]
struct CategoryFilterItem {
    id: usize,
    label: String,
}

impl SelectItem for CategoryFilterItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.label
    }

    fn render(&self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tooltip = self.label.clone();
        div()
            .id(("category-filter-item", self.id))
            .w_full()
            .tooltip(move |window, cx| ComponentTooltip::new(tooltip.clone()).build(window, cx))
            .child(self.label.clone())
    }
}
```

Что важно:

- `title()` остаётся обычной подписью. Она используется в выбранном значении, в поиске и в доступности; тултип не заменяет её.
- `value()` возвращает идентификатор значения. В текущем коде это та же подпись, поэтому обработчик `SelectEvent::Confirm` продолжает искать категорию по названию без изменений.
- `render()` должен вернуть элемент с уникальным `id`: без него `.tooltip()` не сработает. В примере `id` присваивается при создании списка через `enumerate()`.
- `.w_full()` растягивает строку на всю ширину списка, поэтому тултип появляется при наведении не только на текст.
- Текст копируется в замыкание: `render()` вызывается при каждом показе списка.

## 2. Состояние и обновление списка

Тип состояния меняется вместе с типом элемента:

```rust
type CategoryChoice = Entity<SelectState<Vec<CategoryFilterItem>>>;
```

Элементы создаются с индексом, который затем используется как `id` строки:

```rust
let items: Vec<CategoryFilterItem> = labels
    .into_iter()
    .enumerate()
    .map(|(id, label)| CategoryFilterItem { id, label })
    .collect();

cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx))
```

Если данные фильтра перезагружаются, список элементов пересобирается и передаётся через `state.set_items(items, window, cx)`. Подписка на события объявляется с новым типом:

```rust
cx.subscribe_in(
    &category_filter_select,
    window,
    |this: &mut Self, _, event: &SelectEvent<Vec<CategoryFilterItem>>, _, cx| {
        let SelectEvent::Confirm(value) = event;
        this.category_filter = value
            .as_deref()
            .and_then(|label| {
                this.category_filter_labels()
                    .iter()
                    .position(|item| item == label)
            })
            .and_then(|position| position.checked_sub(1))
            .and_then(|position| this.category_filter_ids.get(position).copied());
        this.refresh(cx);
    },
);
```

`value` в обработчике имеет тип `&Option<String>` — это значение, которое вернул `CategoryFilterItem::value()`, а не сам элемент списка.

## 3. Тултип на выбранном значении

Чтобы полный текст был виден, когда список закрыт, `Select` оборачивается в `div` с собственным `id`:

```rust
let category_tooltip = self
    .category_filter_select
    .read(cx)
    .selected_index(cx)
    .map(|ix| ix.row)
    .and_then(|row| {
        let labels = self.category_filter_labels();
        labels.get(row).cloned()
    })
    .filter(|label| label != NONE_LABEL);

div()
    .id("category-filter-select")
    .child(
        Select::new(&self.category_filter_select)
            .placeholder("Категория")
            .small(),
    )
    .when_some(category_tooltip.clone(), |el, tooltip| {
        el.tooltip(move |window, cx| {
            ComponentTooltip::new(tooltip.clone()).build(window, cx)
        })
    })
```

Здесь `when_some` из `gpui_kit::prelude::FluentBuilder`, а `.tooltip()` — метод stateful-элемента. Для значений-заглушек (`Все` и `Не по карте` в фильтре способа получения) тултип не показывается: подпись и так самодостаточна. Для родительской категории в справочниках короткому значению «Нет» соответствует пояснение «Без родительской категории» (`NO_PARENT_TOOLTIP`); оно используется и для строки списка, и для выбранного значения.

## 4. Откуда берётся текст

Полный путь категории строится функцией `category_tree_labels()` из `crates/ui/src/gpui/support.rs`: имена предков соединяются через ` > `. Фильтр добавляет к результату первый вариант «Все», поэтому индекс `0` зарезервирован. Тултип использует ту же готовую подпись, что и `title()`, — отдельное поле с «длинным» названием хранить не нужно.

Если для другого селекта короткая подпись не объясняет значение, задайте отдельную строку пояснения и подставьте её и в `render()`, и в тултип выбранного значения, как это сделано для `Нет` / `Без родительской категории`.

## 5. Чек-лист при переносе на другой селект

1. Создать тип элемента с `#[derive(Clone)]`, уникальным `id` и реализацией `SelectItem`.
2. Оставить в `title()` и `value()` те же значения, что использовались для `Vec<SharedString>`, чтобы не менять обработчик выбора и сохранённые фильтры.
3. Переопределить `render()`, добавив `.id(...)`, `.w_full()` и `.tooltip(...)`.
4. Заменить тип `SelectState`/`SelectEvent` на `Vec<НовыйЭлемент>` и обновить `set_items`.
5. Если выбранное значение тоже может обрезаться, добавить тултип на обёртку вокруг `Select::new(...)` и не показывать его для значения-заглушки.
6. Проверить, что выбор строки по-прежнему работает: элемент `div` не должен перехватывать клик, а его `id` не должен совпадать с `id` других элементов на странице.
7. Если для селекта включён поиск с `highlight_matches(true)`, переопределить и `render_highlighted()`: по умолчанию он рисует `title()` без пользовательской разметки, и тултип пропадёт во время поиска. Текущие фильтры доходов и расходов этот режим не включают.

## См. также

- `docs/page-actions-and-filters.md` — поведение фильтров страниц «Доходы» и «Расходы».
- `docs/form-guidelines.md` — общие правила оформления форм и подсказок.

