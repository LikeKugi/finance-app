---
sessionId: session-261124-214750-racf
---

# Findings

### Result
Определить, где приложение сохраняет выбор темы и какие значения использует. Изменения кода и базы данных не требуются.

# Technical Design

### Current Implementation
- Путь к БД задаётся в `crates/app/src/main.rs`: пользовательский каталог конфигурации, `FinanceApp/data.db`.
- `crates/db/migrations/V1__initial.sql` создаёт таблицу `app_settings` с полями `key` и `value`.
- `crates/ui/src/gpui/settings.rs` читает ключ `theme` при открытии настроек и сохраняет выбор через API из `crates/db/src/lib.rs`.
- Значения предпочтения: `system`, `light`, `dark`; при отсутствии записи используется системная тема.

# Delivery Steps

###   Step 1: Trace theme persistence
Установить путь к базе данных и точный формат сохранённой настройки.
- Проверить выбор пути в `crates/app/src/main.rs`.
- Сверить схему `app_settings` в `crates/db/migrations/V1__initial.sql` и функции чтения/записи в `crates/db/src/lib.rs`.

###   Step 2: Confirm settings behavior
Подтвердить, как выбор темы восстанавливается и сохраняется в интерфейсе.
- Проверить загрузку ключа `theme`, значения предпочтения и поведение по умолчанию в `crates/ui/src/gpui/settings.rs`.
- Сообщить пользователю путь к БД и место хранения настройки; проектные файлы не изменять.