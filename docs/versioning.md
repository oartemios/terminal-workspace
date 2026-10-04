# Версии приложения и сборок

Источник версии приложения — `[package].version` в корневом `Cargo.toml`. Первая версия — `0.1.0`; изменение версии само по себе не создаёт тег или release. `tw --version` и `tw -V` показывают эту версию без открытия Workspace, терминала или plugin store.

Используем SemVer `MAJOR.MINOR.PATCH`. До 1.0 новые возможности и несовместимые изменения приложения увеличивают MINOR, исправления — PATCH. Это локальное решение выпуска; Plugin API (сейчас 0.5), plugin protocol, package manifest и Workspace config имеют отдельные версии. Версия Catalog принадлежит самому custom plugin.

Сборки PR/main/manual получают идентификатор `0.1.0+dev.<12 символов commit>`. Например, архив `terminal-workspace-0.1.0+dev.abcdef012345-aarch64-apple-darwin.tar.gz`. Commit у PR — проверяемый merge commit GitHub. После push тега `v0.1.0` CI создаёт архив `terminal-workspace-0.1.0-aarch64-apple-darwin.tar.gz`; tag обязан совпадать с Cargo version, иначе CI завершается ошибкой до сборки. Prerelease вроде `0.2.0-rc.1` поддерживается тегом `v0.2.0-rc.1`.

В архиве `VERSION` содержит версию приложения, `BUILD_VERSION` — идентификатор сборки, `COMMIT` — полный hash, `TARGET` — платформу. Packaging проверяет, что версия release-бинарника совпадает с Cargo.toml. Установленная команда `tw --version` показывает версию приложения; идентификатор конкретной сборки хранится в архиве. Теговая сборка проходит ту же матрицу установки и PTY, что PR. CI загружает Actions artifacts на 30 дней. После успешной теговой сборки GitHub Release публикуется отдельно по запросу, с теми же архивами и checksums; теги и releases не создаются автоматически.

## Выпуск

1. В отдельной ветке обновить Cargo.toml, root Cargo.lock и lockfile custom plugin (он зависит от root package). Записать изменения версии в CHANGELOG.md; выполнить проверки и открыть PR.
2. После review и явно запрошенного merge синхронизировать main; выбрать commit выпуска с соответствующей Cargo version. Не переносить уже выпущенные теги на другие commits.
3. По запросу выпуска создать annotated tag и отправить его, например:

   ```sh
   git tag -a v0.1.0 -m "Terminal Workspace 0.1.0"
   git push origin v0.1.0
   ```

4. Дождаться всех четырёх jobs в Actions. Скачать их архивы и checksums, проверить SHA-256 и COMMIT/TARGET/VERSION, затем приложить к GitHub Release для того же тега вместе с заметками из CHANGELOG.md. Сборка без `v<version>` остаётся development build.
