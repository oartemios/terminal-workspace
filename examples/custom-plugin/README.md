# Catalog — пример custom plugin

Это отдельный Rust package вне first-party реализации Files. Плагин показывает секцию и заметку, использует opaque locations, default Action, `CommandOutcome::Navigate` и `GroupView::parent`. Core не интерпретирует его секции как каталоги.

Проверка из корня репозитория:

```sh
cargo test --offline --manifest-path examples/custom-plugin/Cargo.toml --target-dir target/custom-plugin
```

Тест проверяет основной Plugin API и TUI одного плагина в двух разных Workspaces. Это независимая упаковка исходного кода, связанного с Core при сборке; динамическая установка или загрузка исполняемого плагина пока не реализована. Используется draft Plugin API v0.1.
