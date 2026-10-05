# Версии приложения и сборок

Источник версии приложения — `[package].version` в корневом `Cargo.toml`. Первая версия — `0.1.0`; изменение версии само по себе не создаёт тег или release. `tw --version` и `tw -V` показывают эту версию без открытия Workspace, терминала или plugin store.

Используем SemVer `MAJOR.MINOR.PATCH`. До 1.0 новые возможности и несовместимые изменения приложения увеличивают MINOR, исправления — PATCH. Это локальное решение выпуска; Plugin API (сейчас 0.6), plugin protocol, package manifest и Workspace config имеют отдельные версии. Версия Catalog принадлежит самому custom plugin.

Сборки PR/main/manual версии 0.2.1 получают идентификатор `0.2.1+dev.<12 символов commit>`. Например, архив `terminal-workspace-0.2.1+dev.abcdef012345-aarch64-apple-darwin.tar.gz`. Commit у PR — проверяемый merge commit GitHub. После push тега `v0.2.1` CI создаёт архив `terminal-workspace-0.2.1-aarch64-apple-darwin.tar.gz`; tag обязан совпадать с Cargo version, иначе CI завершается ошибкой до сборки. Prerelease вроде `0.2.1-rc.1` поддерживается тегом `v0.2.1-rc.1`.

В архиве `VERSION` содержит версию приложения, `BUILD_VERSION` — идентификатор сборки, `COMMIT` — полный hash, `TARGET` — платформу. Packaging проверяет, что версия release-бинарника совпадает с Cargo.toml. Установленная команда `tw --version` показывает версию приложения; идентификатор конкретной сборки хранится в архиве. Теговая сборка проходит ту же матрицу установки и PTY, что PR. CI загружает Actions artifacts на 30 дней. После успешной теговой сборки отдельный release job автоматически публикует GitHub Release с теми же архивами и checksums. Создание версии, merge и push тега по-прежнему требуют явного решения о выпуске; PR/main/manual builds не публикуют releases.

## Выпуск

1. В отдельной ветке обновить Cargo.toml, root Cargo.lock и lockfile custom plugin (он зависит от root package). Записать изменения версии в CHANGELOG.md; выполнить проверки и открыть PR.
2. После review и явно запрошенного merge синхронизировать main; выбрать commit выпуска с соответствующей Cargo version. Не переносить уже выпущенные теги на другие commits.
3. По запросу выпуска создать annotated tag и отправить его, например:

   ```sh
   git tag -a v0.2.1 -m "Terminal Workspace 0.2.1"
   git push origin v0.2.1
   ```

4. После push тега Actions выполняет четыре платформенные сборки, установку и PTY-проверки. Только после их успеха release job скачивает artifacts текущего run, проверяет полный комплект из восьми файлов, SHA-256, COMMIT/TARGET/VERSION/BUILD_VERSION и соответствие remote tag проверяемому commit. Заметки берутся из единственной непустой секции `## <version> — <date>` в CHANGELOG.md. Затем job создаёт draft, загружает файлы, скачивает их обратно и сравнивает байты перед публикацией. Prerelease-теги получают признак prerelease; выбор latest остаётся стандартным поведением GitHub. Ожидать CI вручную и переносить файлы не требуется.

## Повтор после ошибки

Если платформа или проверка artifacts завершилась ошибкой, release не публикуется. В Actions можно повторить failed jobs того же тегового run; release job использует artifacts этого run (хранятся 30 дней). При сбое загрузки или проверки уже загруженных файлов остаётся draft. Повтор job обновит заметки draft, заменит одноимённые файлы и снова проверит полный комплект перед публикацией. Неожиданные файлы draft блокируют публикацию: удалить лишние файлы после проверки причины и повторить job.

Уже опубликованный релиз повторный job оставляет без изменений; исправления выпускаются новой версией и тегом. Ошибки доступа/API не считаются отсутствием релиза и завершают job. Не переносить тег для исправления сбоя. При истечении срока хранения artifacts потребуется новый полный run для того же неизменного тега.

Только release job имеет `contents: write` и `actions: read`; build jobs сохраняют `contents: read`. Release job запускается исключительно на push `refs/tags/v*`, а не на PR или workflow_dispatch. Теговые runs не отменяют друг друга посреди публикации. Это решение CI, не изменение Core или Plugin API.

Локальные проверки без публикации: `python3 tests/release.py`. Для проверки реальных теговых artifacts на checkout соответствующего commit: `GITHUB_REF=refs/tags/v<version> GITHUB_SHA=<commit> python3 scripts/release.py --artifacts <directory> --check-only`.
