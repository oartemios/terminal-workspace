# Changelog

## Unreleased

- Введено версионирование приложения от Cargo.toml: `tw --version` / `-V`.
- Установочные архивы содержат версию приложения и идентификатор commit; CI проверяет теги `v<version>`.

## База 0.1.0 (ещё не выпущена тегом)

- Terminal-first интерфейс, Workspace, общий CommandRegistry и клавиатурная навигация.
- Процессный runtime и независимо упакованный custom plugin; lifecycle и permission gates без OS sandbox.
- Files preview, Markdown, поиск и переход по исходным строкам; Git Status, Branches и diff.
- Проверяемые установочные сборки Linux/macOS на x86_64 и ARM64.
