# 0009. CI и локально устанавливаемые артефакты

Дата: 4 октября 2026. Статус: локальное решение реализации.

Пользователь установил правило: новые изменения идут через ветки и Pull Requests. Оно записано в AGENTS.md. Этот workflow не устанавливает серверный branch protection; он запускает проверки PR и main, а merge остаётся отдельной операцией.

Сценарий: скачать сборку выбранного commit, проверить checksum, распаковать и установить `tw` без Rust и sudo. Core/Plugin API и командный маршрут приложения не меняются. Installer копирует executable в пользовательский prefix; Files и Git остаются пакетами существующего runtime, не получают особый путь активации или permissions.

Текущий терминальный слой, process groups, poll и executable permissions используют Unix API. Поэтому первая матрица — Linux GNU x86_64/ARM64 и macOS Intel/Apple Silicon. Native Windows требует отдельного портирования и не заявляется поддерживаемым. Linux собирается на Ubuntu 22.04 (glibc 2.35); macOS deployment target 11.0, тестовые runner ОС 14/15. Проверки старых macOS, musl, code signing/notarization и публикация GitHub Releases не входят в эту итерацию.

Нативные runners позволяют выполнить реальные тесты на каждой архитектуре вместо одной cross compilation. Rust stable с Cargo.lock — обратимый выбор CI, не новый MSRV-контракт; указанная в Cargo.toml версия 1.74 в этом workflow отдельно не проверяется. Action versions и runner labels явно указаны; обновляются отдельным PR.

Каждая платформа проходит fmt, Clippy, all-targets tests, independently packaged custom plugin, release build, упаковку, SHA-256, распаковку, установку в временный prefix и полный PTY-сценарий установленного бинарника. `TW_TEST_BINARY` позволяет использовать существующий PTY suite для release, не меняя defaults локального запуска. Архив tar сохраняет executable permissions внутри GitHub artifact ZIP. В архиве есть TARGET, COMMIT, README и installer; checksum рядом. SHA-256 проверяет целостность, не заменяет подпись происхождения. Артефакты хранятся 30 дней и доступны на PR, main и manual runs. Минимальные права workflow — contents:read; secrets не нужны.

Глобальные пакеты плагинов имеют отдельный lifecycle и не перезаписываются installer. README описывает отдельное обновление существующих Files/Git executable через package CLI.

Первый CI прогон прошёл Linux x86_64/ARM64 и macOS ARM64; macOS Intel получил timeouts Git worker при параллельном выполнении пяти process-heavy тестов. CI запускает Rust tests последовательно (`RUST_TEST_THREADS=1`) и заранее вызывает системный Git. Deadline runtime остаётся 1 s; проверки и их assertions не отключаются.
