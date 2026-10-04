# Catalog — пример custom plugin

Независимый Rust package вне first-party дерева. Секции и заметки сохраняют нативную семантику; opaque locations, default Action, Navigation и parent используют основной Plugin API 0.4. Core не интерпретирует секции как каталоги.

## Сборка и установка

Из корня репозитория:

```sh
cargo build --offline --manifest-path examples/custom-plugin/Cargo.toml --target-dir target/custom-plugin
target/custom-plugin/debug/tw-example-catalog --package /tmp/catalog-package
target/debug/tw plugins install /tmp/catalog-package
```

Каталог назначения должен быть новым. Manifest и executable устанавливаются в глобальный store, вне Workspace. Host не пересобирается. В TUI:

```text
:core.plugin.trust catalog
:core.plugin.enable catalog
```

Trust разрешает запуск native executable с правами пользователя: OS sandbox отсутствует. Catalog не запрашивает permissions. Files использует тот же process SDK и protocol. Disable/suspend прекращают worker; uninstall отдельно удаляет пакет, сохранив settings.

## Настройки и управление

В `.terminal-workspace.json` settings плагина `catalog` принимают строку `greeting`; Read note отображает её из `Workspace::plugin_settings`. Настройки независимы для разных проектов.

Enter/`l` открывают секцию, `h`/Backspace возвращают к индексу. Локальный `p` читает заметку в Introduction (View scope). Действие также доступно через `a`, palette и `:catalog.read`. Прежние defaults `o/u/nrd` отсутствуют; overrides позволяют собственную раскладку.

## Проверка

```sh
cargo test --offline --manifest-path examples/custom-plugin/Cargo.toml --target-dir target/custom-plugin
```

Четыре теста проверяют API, TUI, settings/activation после restart и фактическую упаковку executable, установку, trust, четыре командных маршрута, два Workspace, suspend/enable и uninstall. [Контракт protocol](../../docs/plugin-protocol.md) и [решение runtime](../../docs/decisions/0006-executable-plugin-runtime.md) описывают пределы исполнения и доступа.
