# Catalog — пример custom plugin

Это отдельный Rust package вне first-party реализации Files. Плагин показывает секцию и заметку, использует opaque locations, default Action, `CommandOutcome::Navigate` и `GroupView::parent`. Core не интерпретирует его секции как каталоги.

Проверка из корня репозитория:

```sh
cargo test --offline --manifest-path examples/custom-plugin/Cargo.toml --target-dir target/custom-plugin
```

Тест проверяет основной Plugin API и TUI одного плагина в двух разных Workspaces. Это независимая упаковка исходного кода, связанного с Core при сборке; динамическая установка или загрузка исполняемого плагина пока не реализована. Используется draft Plugin API v0.3.

В project-local settings для `catalog` можно задать строку `greeting`; Core передаёт её через `Workspace::plugin_settings`, а Catalog использует в результате Read note. Отдельная проверка открывает два временных проекта с разными greeting, сохраняет отключение и повторное включение, проверяя сохранность settings после restart.

Catalog использует общую навигацию: Enter/`l` открывают секцию, `h`/Backspace возвращают к индексу. Локальный `p` читает заметку только внутри Introduction (View scope). Прежние defaults `o/u/nrd` удалены. Тест переключает два проекта в одном App без повторной установки плагина; проверяет settings, возврат к сохранённому контексту, справку, область действия binding и одинаковый результат `p`, `:command`, palette и Action. Актуальные правила описаны в [решении итерации 2.1](../../docs/decisions/0005-keyboard-defaults.md).
