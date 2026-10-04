# 0007. Git-плагин: локальный просмотр

Дата: 4 октября 2026. Статус: решение реализации первой Git-итерации; не исходный продуктовый контракт.

## Сценарий и граница

Workspace → Git → Status → изменённый файл → diff; Workspace → Git → Branches → локальная ветка → последний commit. Git-логика находится в `src/git.rs`. Core не трактует Git-коды состояния, пути rename, refs или diff. `Item.kind` сохраняет собственные виды `changed-file` и `branch`; diff и сведения о ветке — результаты `Block`, а не замена объектов.

Git упаковывается командой `tw plugins package-git <новый-каталог>`. Executable — копия `tw` с режимом `--serve-git`, как Files с `--serve-files`. Пакет использует обычные `write_package`, `PackageStore`, `ProcessPlugin` и `serve_plugin`; привилегированного пути в TUI нет. Установка, trust, Workspace activation и permissions выполняются явно. Автоматический bootstrap Git не добавлен. Публичный Plugin API 0.4 и protocol 1 не изменены; independently packaged Catalog продолжает работать без изменений.

## Команды и данные

Стабильные группы `status` и `branches` существуют и при отсутствии Items. `git.diff` и `git.branch` требуют выбранный Item; явный аргумент командной строки задаёт тот же opaque ItemId. Contextual Action/default Enter, scoped `p`, палитра и `:commands` обращаются к тем же CommandId. `p` — изменяемый default этой итерации: diff в Status, сведения в Branches. Refresh, фильтр и сортировка используют существующие механизмы TUI.

Локальный установленный Git вызывается через `std::process::Command`, без shell и новых crates. Это обратимый технический выбор. Status использует [porcelain v1 с NUL-разделителями](https://git-scm.com/docs/git-status#_porcelain_format_version_1): коды индекс/рабочее дерево сохраняются в подписи, путь назначения rename становится ItemId, исходный путь сохраняется для diff. Имена с пробелами, newline и ведущим дефисом не разбираются по пробелам/строкам. Non-UTF-8 выдаёт ошибку вместо изменения идентичности объекта через lossy conversion.

Branches использует `for-each-ref refs/heads/`, полный ref как ItemId и текущую ветку как `*` в подписи. Remote branches пока не выводятся. В unborn repo локальных refs ещё нет; в detached HEAD нет текущей отмеченной ветки. Сведения — последний commit выбранного существующего локального ref.

[`git diff`](https://git-scm.com/docs/git-diff) выполняется отдельно для индекса (`--cached`) и рабочего дерева; результат содержит непустые секции. Для rename передаются оба пути. Untracked файл сравнивается с `/dev/null` через `--no-index`; exit code 1 здесь означает найденные различия. В репозитории без HEAD cached diff поддерживается самим Git. Перед выполнением сверяется актуальный Status/Branches: устаревший Item получает понятную ошибку с предложением refresh. Untracked symlink за границу root отклоняется SDK path helper.

## Исполнение и ограничения

Плагин объявляет WorkspaceRead и Process; host проверяет оба до предметного вызова. Выбраны только операции просмотра: stage/unstage, commit, checkout, fetch и push не добавлены. Git запускается с закрытым stdin, отключёнными pager, цветом, внешним diff, textconv и fsmonitor. `GIT_OPTIONAL_LOCKS=0` избегает optional записи индекса при status. Pathspecs literal, аргументы файлов идут после `--`, пользовательские refs проходят проверку списка локальных веток. Системный/глобальный config выключен, environment очищен; фиксированный путь поиска Git `/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin` не передаёт пользовательские environment variables. Произвольный executable path пока не настраивается.

Каждый stdout/stderr ограничен 128 KiB, pipes читаются одновременно. Превышение — доменная ошибка, без вывода частичного diff; worker продолжает обслуживать небольшие запросы. Общий предел protocol 1 MiB сохраняется. В production worker имеет deadline runtime 1 s; timeout завершает его группу вместе с Git. Вызовы остаются синхронными и могут задержать ввод до deadline. Сам linked Plugin не реализует отдельного timeout: прямое in-process использование предназначено для SDK/fixtures.

Workspace должен быть корнем рабочего репозитория. Открытие вложенного каталога выдаёт просьбу открыть корень; bare repositories не поддержаны. Это ограничение выбранной реализации, позволяющее сохранить точные repository-relative ItemId без доступа к другим каталогам проекта. Worktree metadata может находиться вне root: WorkspaceRead/Process — host gates, не OS sandbox. Локальная конфигурация Git и сам установленный Git остаются доверенными. Не заявляется filesystem isolation или kernel network enforcement. Предметные команды этой реализации не обращаются к сети.

Нет отдельного rich diff renderer, автоматического preview при движении, background refresh, remote refs или history browser. Текст должен быть UTF-8; Git обычно выводит summary для binary files. Слишком большой diff возвращает ошибку. Пустое состояние использует существующее `No entries`.

## Проверки

`tests/git_flow.rs` устанавливает Git executable в временный global store и использует общий runtime: staged/unstaged, rename, удаление, untracked, literal pathspec, имена с пробелами/Unicode/newline/ведущим дефисом; четыре маршрута diff и scoped binding веток; два Workspace с отдельной активацией/permissions; чистая группа и unborn/detached HEAD; Process revoke и suspend с доступным Files; oversized diff и следующий успешный запрос.

Unix PTY проходит package CLI, явную активацию/permissions, Status → Action → diff, `p`, Branches, переключение двух Workspaces, suspend Git и последующий Files preview, восстановление терминала. Общие Core/runtime/TUI tests и independently packaged Catalog проверяют отсутствие регрессий. Performance-цели не объявляются достигнутыми: единичный debug PTY первого кадра 676.9 ms (выше <300 ms), открытие/возврат каталога Files по 1.0 ms; полноценные release benchmarks остаются этапом 7.
