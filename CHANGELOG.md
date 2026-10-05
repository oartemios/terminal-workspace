# Changelog

## Unreleased

- GitHub Release автоматически публикуется после успешных теговых сборок всех четырёх платформ: проверка архивов, заметки из CHANGELOG и восстановление незавершённого draft.

## 0.2.0 — 2026-10-05

- Startup/handshake, view, contextual Actions, команды и restart executable plugins в TUI выполняются в фоне; навигация остаётся доступной.
- Отмена при lifecycle/Workspace changes завершает worker process group и не допускает поздних результатов в новый контекст.
- Draft Plugin API 0.6 добавляет polling-контракт; package/protocol 1 и manifests API 0.4/0.5 остаются совместимыми.
- ROADMAP синхронизирован с issues: приоритеты, зависимости и performance-ограничения выпуска.

Ограничения 0.2.0: прототип до MVP; GitHub, кэш и refresh strategies ещё не реализованы. Request/handshake deadlines остаются 1 s/5 s; linked SDK default синхронен. OS sandbox отсутствует. Производительность по целям MVP не подтверждена.

## 0.1.0 — 2026-10-04

- Введено версионирование приложения от Cargo.toml: `tw --version` / `-V`.
- Установочные архивы содержат версию приложения и идентификатор commit; CI проверяет теги `v<version>`.
- Terminal-first интерфейс, Workspace, общий CommandRegistry и клавиатурная навигация.
- Процессный runtime и независимо упакованный custom plugin; lifecycle и permission gates без OS sandbox.
- Files preview, Markdown, поиск и переход по исходным строкам; Git Status, Branches и diff.
- Проверяемые установочные сборки Linux/macOS на x86_64 и ARM64.
