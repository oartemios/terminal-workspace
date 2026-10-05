# Terminal Workspace — roadmap до MVP

Обновлено: 5 октября 2026.

Цель MVP: полезная без AI терминальная рабочая среда для локального проекта с Files, Git, GitHub и независимо устанавливаемым custom plugin. Каждый этап оставляет работающий сквозной сценарий.

План опирается на [AGENTS.md](AGENTS.md), [контекст проекта](Базовый%20контекст%20проекта.md) и [Draft требований](Базовые%20продуктовые%20требования.md). Конкретные crates, runtime, форматы и структура пакетов — решения реализации в `docs/decisions/`, а не исходные продуктовые требования.

## Где ведётся план

- **AGENTS.md** — принятые продуктовые и архитектурные ограничения.
- **ROADMAP.md** — карта этапов, текущего состояния и порядка работ.
- **[Issues](https://github.com/oartemios/terminal-workspace/issues)** — источник актуального scope, приоритетов, зависимостей и критериев завершения конкретных задач.
- **[Трекер MVP #16](https://github.com/oartemios/terminal-workspace/issues/16)** — статус оставшихся задач и готовности MVP.
- **Pull Requests** — реализация, результаты проверок и связь с задачей.

Приоритеты: P0 блокирует готовность MVP; P1 — важное улучшение архитектуры, UX или качества; P2 — расширение после базового MVP. Подробные acceptance criteria поддерживаются в issues.

## Что уже работает

| Этап | Результат и проверки | Документация |
| --- | --- | --- |
| Workspace и активация | Project-local настройки и permissions, постоянное и временное отключение; проверены restart, ошибки конфигурации и независимость проектов. | [Решение 0003](docs/decisions/0003-workspace-configuration.md) |
| Переключение Workspace и ввод | Восстановление контекста в сессии, scoped multi-key bindings и overrides; Files и custom Catalog проверены в двух Workspaces, включая общий маршрут команд и текстовый ввод. | [Решение 0004](docs/decisions/0004-workspace-switching-and-bindings.md), [defaults клавиш](docs/decisions/0005-keyboard-defaults.md) |
| Пакеты, runtime и permissions | Глобальная установка, discovery, trust, lifecycle и uninstall executable plugins; проверены custom plugin, отказы permissions, сбои, таймауты и завершение процессов. | [Решение 0006](docs/decisions/0006-executable-plugin-runtime.md), [протокол](docs/plugin-protocol.md) |
| Files и просмотр output | Browse, вложенная навигация, фильтр, сортировка, refresh и preview; общий viewer с поиском, переходом к строке и базовым Markdown/source. Проверены командные маршруты, Unicode, resize и PTY. | [Решение 0008](docs/decisions/0008-output-viewer.md), [Plugin API 0.5](docs/plugin-api.md) |
| Git | Устанавливаемый read-only плагин: Status, Branches, diff и сведения о ветке. Проверены два репозитория, четыре маршрута команд, пустые группы, специальные пути и доступность Files после ошибки Git. | [Решение 0007](docs/decisions/0007-read-only-git-plugin.md) |
| Сборки и установка | CI для Linux/macOS на x86_64/ARM64, устанавливаемые архивы и версии сборок; workflow включает Rust/custom-plugin проверки и PTY установленного release-бинарника. | [Решение 0009](docs/decisions/0009-ci-artifacts.md), [версии и выпуск](docs/versioning.md) |

Это прототип. GitHub и асинхронное обновление ещё не реализованы; Files/Git пока находятся в основном crate. Requests синхронны с deadline, поэтому медленный worker может задерживать TUI. Native executable требует trust и не изолирован OS sandbox: host проверяет объявленные permissions, но не ограничивает весь прямой доступ процесса к ОС. Производительность по целям MVP пока не подтверждена. Актуальные пользовательские ограничения описаны в [README](README.md).

## Ближайшие этапы — P0

| Этап | Пользовательский результат | Зависимости |
| --- | --- | --- |
| [#4 — Неблокирующий runtime](https://github.com/oartemios/terminal-workspace/issues/4) | Навигация и Files остаются отзывчивыми при медленном startup/handshake, первоначальной загрузке групп и фоновых операциях. Отмена и проверка поколения не допускают результатов старого Workspace. | Следующая задача. |
| [#5 — Кэш и refresh](https://github.com/oartemios/terminal-workspace/issues/5) | Кэш показывается сразу, обновление идёт в фоне; видны stale/offline/error состояния. | После #4. Manual/on-focus/interval refresh не требуют #9; событийный refresh отложен до event bus. |
| [#7 — Отделение first-party плагинов от Core](https://github.com/oartemios/terminal-workspace/issues/7) | Files/Git и публичная SDK/runtime граница отделены; новый сетевой плагин использует тот же доступный custom plugins путь. | До реализации #6; можно выполнять независимо от #4/#5. Точный layout выбирается при реализации. |
| [#6 — GitHub: PR и Issues](https://github.com/oartemios/terminal-workspace/issues/6) | Нативные PR/Issue, клавиатурные Actions, явная авторизация и permissions; slow/offline/auth ошибки не мешают локальной работе. | После #4, #5 и #7. |
| [#15 — Сквозная приёмка MVP](https://github.com/oartemios/terminal-workspace/issues/15) | Files/Git/GitHub/custom plugin проходят полный keyboard-only lifecycle в нескольких Workspaces; проверки, ограничения и решение о выпуске зафиксированы. | После предыдущих P0; использует измерения #12. |

Основная последовательность: **#4 → #5 → #6 → #15**, с дополнительной обязательной зависимостью **#7 → #6**. Доменные данные и логика остаются у плагинов; Core предоставляет общий runtime, отмену, scheduling, permissions и отображение состояния.

## Архитектура, UX и качество — P1

| Задача | Место в плане |
| --- | --- |
| [#8 — Определение применимости плагина к Workspace](https://github.com/oartemios/terminal-workspace/issues/8) | Optional supported/unsupported/unknown через публичный API; обнаружение не включает плагин автоматически. Полезно вместе с #13. |
| [#9 — Core event bus и подписки плагинов](https://github.com/oartemios/terminal-workspace/issues/9) | Общая основа для событийного refresh из #5 и будущих потребителей. Не блокирует manual/on-focus/interval refresh или базовый MVP. |
| [#10 — Files Search и Recent](https://github.com/oartemios/terminal-workspace/issues/10) | Дополняет Browse; длительный поиск использует механизм #4. |
| [#11 — Git: изменяющие действия и history](https://github.com/oartemios/terminal-workspace/issues/11) | Небольшой сквозной сценарий изменения Git и просмотра history через общий командный маршрут и permissions. |
| [#12 — Воспроизводимые performance-измерения](https://github.com/oartemios/terminal-workspace/issues/12) | Baseline можно снять сейчас; финальные сценарии учитывают #4/#5/#6. Оптимизация следует измерениям. |
| [#13 — Keyboard-first выбор плагинов Workspace](https://github.com/oartemios/terminal-workspace/issues/13) | Выбор нескольких плагинов, подтверждение/отмена и недостающие permissions; может появиться до #8. Installation, trust и session suspend остаются отдельными операциями. |

Эти задачи не должны задерживать основную P0-цепочку. Перед закрытием трекера #16 оставшиеся P1 должны быть завершены либо явно вынесены за MVP с объяснением; выполненные части и follow-up фиксируются в issues.

## Производительность и выпуск

Цели сохраняются: startup без сетевых плагинов < 300 ms, открытие локальной группы < 100 ms, воспринимаемая задержка навигации < 16–50 ms. [#12](https://github.com/oartemios/terminal-workspace/issues/12) задаёт воспроизводимые измерения release-сборок с условиями запуска; единичный debug PTY run не подтверждает достижение целей.

По принятому решению недостижение этих latency-целей **не блокирует базовый MVP**. Измеренный результат, окружение, влияние на пользователя и follow-up документируются в итогах [#15](https://github.com/oartemios/terminal-workspace/issues/15), README и release notes. Измерение и честный отчёт остаются обязательными.

Это исключение не снимает требования #4/#5: медленная сеть не блокирует навигацию, доступный кэш показывается сразу, обновление выполняется асинхронно. Приёмка также подтверждает lifecycle, permissions, изоляцию ошибок, работу offline и общий публичный путь custom plugin; подробный checklist находится в #15.

## Отложенные расширения — P2

| Задача | Направление |
| --- | --- |
| [#14 — Структурированные context providers](https://github.com/oartemios/terminal-workspace/issues/14) | Optional публичный контракт для потребителей контекста, включая будущий AI plugin; без AI-зависимости и универсального WorkItem. |
| [#17 — Preview изображений в Files](https://github.com/oartemios/terminal-workspace/issues/17) | Расширение `files.preview` с терминальным fallback, лимитами и очисткой изображения. Форматы и протокол вывода выбираются отдельно. |
| [#18 — Русская локализация UI](https://github.com/oartemios/terminal-workspace/issues/18) | Подписи Core и плагинов, поиск локализованных команд и fallback; идентификаторы и пользовательский текст сохраняются. |
| [#20 — Русская раскладка клавиш](https://github.com/oartemios/terminal-workspace/issues/20) | Позиционные ЙЦУКЕН-эквиваленты bindings через тот же CommandId; ввод текста остаётся буквальным. Независимо от языка UI и #18. |
| [#19 — Workspace-agnostic Writing plugin](https://github.com/oartemios/terminal-workspace/issues/19) | Небольшой сквозной сценарий для разных писательских проектов через основной Plugin API. Точный scope выбирается после проверки MVP. |

P2 не блокирует выпуск базового MVP. Отдельно запланированы скриншоты реальных Files, Markdown preview, Git Status/diff и palette/Actions в README на воспроизводимом примере без личных данных; место docs-задачи в очереди пока не определено.

AI остаётся optional. Marketplace, облачная синхронизация, совместная работа, универсальный task manager/WorkItem, RAG и сложная agent orchestration не входят в план базового MVP.

## Как поддерживать план

Выбирать готовое к работе issue, уточнять небольшой сквозной сценарий и выполнять изменение в тематической ветке. PR связывать с issue и указывать результаты относящихся к изменению проверок; merge выполнять только по явному запросу пользователя. После merge закрывать завершённую задачу и обновлять трекер #16. ROADMAP обновлять при завершении этапа или изменении последовательности, без копирования подробных acceptance criteria из issues.

Изменения scope, приоритетов и зависимостей сначала фиксировать в issues и отражать здесь, если меняется карта этапов. Технические решения записывать в `docs/decisions/`; при расширении публичного API проверять независимо упакованный custom plugin в нескольких Workspaces. Новые продуктовые обязательства принимать отдельно.
