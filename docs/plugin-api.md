# Plugin API v0.6 — draft

`PLUGIN_API_VERSION = "0.6"` обозначает исходный Rust-контракт. Files и независимо упакованный Catalog используют публичный SDK; production TUI загружает оба через одинаковый процессный runtime. Стабильного Rust ABI нет. Для внешних executable packages отдельно версионируются package format 1 и [JSON-lines protocol 1](plugin-protocol.md).

## Данные и вызовы

- `Plugin::id/name` возвращают borrowed `&str`. Groups — стабильные области; Commands регистрируются в namespace плагина. Capabilities необязательны.
- `items` задаёт корневые Items Group. `view` расширяет загрузку вложенными контекстами; default поддерживает только корень.
- `GroupView` содержит Items, title, opaque location, parent invocation и command_defaults. Item id/kind имеют нативный смысл; Core не интерпретирует их как пути. ID элементов одного view уникальны.
- `item_icon` задаёт символ списка; Core заменяет небезопасные/неодноклеточные символы на fallback.
- `actions` задаёт действия Item; `try_actions` позволяет сообщить ошибку. Максимум одно действие default: Enter/`l` вызывает его, иначе открывает Actions. CommandId и invocation.id должны совпадать и принадлежать плагину.
- `Block.format` объявляет `Text` (default) или `Markdown`; source content сохраняется для поиска, source-line навигации и переключения presentation/source. Core предоставляет общий безопасный terminal renderer.
- `execute` возвращает `Output(Block)` либо `Navigate(Navigation)` с группой, opaque location и необязательным selected ItemId. `WorkspaceChanged` и `View(ViewRequest)` зарезервированы для Core и отклоняются от плагинов.

`CommandInvocation.args` независим от Item. App проверяет регистрацию, activation и permissions; bindings, `:commands`, palette и Actions сходятся к одному маршруту. В TUI явный аргумент имеет приоритет, затем command_defaults view, затем выбранный Item для команды его плагина, требующей Item. Ошибка загрузки нового view сохраняет текущий контекст. Фильтр, порядок и выбор сохраняются для локации в сессии.

Files нормализует пути внутри Workspace и задаёт default Action для каталогов. `files.open/parent` возвращают Navigation; `files.preview/path` — Blocks. Catalog использует те же контракты для секций и заметок без Files-специфичного пути в Core.

## Workspace и конфигурация

`App::open` читает project-local конфигурацию; `App::new` создаёт сессию без записи. `plugin_settings(id)` возвращает JSON settings плагина. `grant_default` применяет явный default только без сохранённого решения; grant/revoke сохраняют явное решение. Повреждённая конфигурация блокирует запись до исправления и перезапуска. Unknown fields сохраняются.

При каждом вызове передаётся Workspace. Процессный контекст содержит canonical root, settings только текущего плагина и grants; UI overrides и настройки соседних плагинов не передаются. Worker завершается при переключении root. Один пакет работает с разными проектами; activation, permissions, settings и suspend независимы. `core.workspace.open` принимает путь, `core.workspace.previous` возвращает к прошлому проекту; подготовка нового проекта предшествует замене текущего.

## Пакеты и lifecycle

`runtime::write_package` создаёт manifest и копирует executable; entrypoint worker вызывает `runtime::serve_plugin`. `PackageStore` устанавливает/обнаруживает пакеты без выполнения кода. `App::load_packages(store, defaults)` подключает обнаруженные пакеты; defaults задаются явно. Installation, Workspace/session activation, availability и connection — отдельные состояния. `App::install` с linked trait остаётся для доверенного SDK usage и тестов.

API 0.4 добавляет serde DTO и hooks `start`, `stop`, `runtime_status`, `installation_present`, `try_actions`. Start должен быть идемпотентным. Процессный adapter запускает worker после trust и permissions, проверяет handshake descriptor; SDK вызывает native start перед предметными операциями. Stop worker — завершение process group и освобождение pipe/child. Native cleanup hook не гарантирован при жёстком завершении. В linked usage Core вызывает stop, но не может принудительно остановить произвольные threads.

Отказ обычной операции сохраняет соединение; transport fault завершает worker, показывает failed и требует явного restart/enable. Limits и deadline описаны в protocol. Production TUI использует polling-контракт API 0.6: startup/handshake, view, Actions и команды executable plugins выполняются вне input/render loop. Синхронные методы App остаются для коротких локальных вызовов и доверенного linked SDK usage.

## Permissions и trust

Категории: `WorkspaceRead`, `WorkspaceWrite`, `Process`, `Network`, `Credentials`, `Environment`. Host требует все объявленные permissions до запуска и предметных вызовов. Недоверенный пакет не выполняется до отдельного trust. Отзыв permissions/trust, suspend, disable, uninstall и Drop App завершают worker текущего приложения.

Host очищает environment; передаёт только manifest allowlists при соответствующих grants. Credentials здесь — named environment forwarding, без keychain/vault и записи значений в project-local JSON. Runtime `Workspace.read_path/write_path` проверяет grants и принадлежность корню, включая symlink escape. Эти helpers не защищают от filesystem races.

**OS sandbox отсутствует.** Trusted native executable сохраняет права пользователя и может обращаться к ОС напрямую, обходя SDK и неполную декларацию permissions. Process/Network grants — host gates, не syscall enforcement. Подробнее: [решение runtime](decisions/0006-executable-plugin-runtime.md).

## Core-команды

Все выполняются через CommandRegistry:

- `core.plugins`, `core.plugins.discover` — статус и повторный discovery.
- `core.plugin.install <package-dir>`, `uninstall <id>` — установка и отдельное удаление; настройки проекта сохраняются.
- `core.plugin.trust/untrust/restart <id>` — trust и повторный запуск.
- `core.plugin.enable/disable/suspend <id>` — Workspace/session activation.
- `core.permissions <id>`, `core.permission.grant/revoke <id> <Permission>` — grants. Старые grant-read/revoke-read нормализуются к общему маршруту WorkspaceRead.

В palette/bindings/Actions цель задаёт текущая вкладка без Item. Явный plugin id имеет приоритет. При недостатке аргументов TUI открывает command prompt для пути или Permission. Uninstall не равен disable; discovery не активирует custom plugin автоматически.

## Bindings

`Plugin::keybindings` default — пустой список. `KeyBinding { keys, command_id, scope }` использует Plugin, Group или View scope собственного namespace. Global bindings принадлежат Core. Overrides приоритетнее defaults; конфликты отключаются с диагностикой. Поддерживаются печатные символы без пробелов. Правила: [решение этапа 2](decisions/0004-workspace-switching-and-bindings.md).

Defaults остаются изменяемыми: Files — локальный `p` для preview; Catalog — `p` для Read note только в Introduction. Открытие и parent используют общую грамматику `h/j/k/l`, Enter и Backspace. [Итерация 2.1](decisions/0005-keyboard-defaults.md) описывает раскладку; ранее удалённые aliases можно вернуть overrides.

Изменения относительно 0.3: сериализуемые DTO, дополнительные permissions, lifecycle/status hooks, borrowed id/name, процессный SDK и фактическая установка пакета без пересборки host. Внешний контракт остаётся draft; events и refresh strategies появятся отдельно.

API 0.5 добавляет ContentFormat и Core-only ViewRequest. Rust Block literals должны явно задавать format. Новый host принимает manifests API 0.4 и 0.5; в старых JSON Blocks отсутствие format означает Text. Package/protocol остаются version 1. `core.view.find/goto/next/previous/source` управляют открытым output, не требуют Item и не вызывают plugin. `/`, `g`, `n/N`, `v`, `:120` и адресуемые команды сходятся к этому маршруту.

## API 0.6: polling фоновой работы

`runtime::BackgroundRequest` содержит `Start`, `View { group, location }`, `Actions { group, location, item }` или `Execute(CommandInvocation)`. `BackgroundResponse` содержит `Started`, `View(GroupView)`, `Actions { view, actions }` или `Executed(CommandOutcome)`. View при Actions повторно проверяет существование Item; host проверяет также уникальность IDs и ownership ссылок этого view.

`Plugin::poll_background(workspace, permissions, request)` возвращает `std::task::Poll<Result<BackgroundResponse, String>>`. Capability необязательна: default выполняет обычные hooks синхронно для доверенных linked implementations. ProcessPlugin предоставляет неблокирующую реализацию для любого установленного executable package, включая independently packaged custom plugins; native plugin не должен реализовывать отдельный async API. Linked default не гарантирует responsiveness для медленного native кода.

`App::poll_background(plugin_id, request)` применяет activation, permissions и те же проверки View/Actions/Outcome, что синхронный API. `App::poll_invoke(invocation)` — polling-маршрут CommandRegistry: зарегистрированная команда исполняется с тем же CommandId и аргументами из binding, command line, palette и Action. Core-команды непосредственные, кроме polling restart, включающего handshake. Синхронный `App::invoke` сохраняет прежний контракт.

Host повторяет один request до `Ready`; `Pending` означает ongoing работу, а не пустой успешный результат. После `Ready` следующий вызов начинает новую операцию. На plugin допускается один pending request и один сохранённый completion. Повторный pending request coalesces; другой request ждёт занятого worker без растущей очереди, затем supersedes прежний результат. Не смешивать synchronous и polling calls, пока у plugin есть pending work: sync path возвращает диагностическую ошибку.

Executable adapter сохраняет живой worker между операциями. Отдельный cancellation handle принадлежит каждой фоновой операции; stop завершает process group и удаляет pending/completed state. Suspend, disable, permission/trust revoke, Workspace switch, uninstall и Drop App используют этот lifecycle. Старый поток не может опубликовать completion в новую generation. TUI дополнительно проверяет Workspace root и собственный epoch перед применением результата; смена plugin/group или новый пользовательский контекст не заменяются поздним response.

`Connection::Loading` отличает выполняемую фоновую операцию от Running/Disconnected/Failed. TUI показывает Loading в statusline, сохраняет клавиатурную навигацию и опрашивает completion через `Ui::tick`. Отмена UI-перехода скрывает его поздний результат; она не обещает rollback уже выполняющейся команды. Lifecycle cancellation завершает процесс, но также не откатывает совершённые им внешние действия.

Wire operations, package format и protocol остаются version 1; host 0.6 принимает manifests API 0.4, 0.5 и 0.6. Кэш, refresh strategies и event subscriptions здесь не добавлены. [Решение фонового runtime](decisions/0010-background-plugin-runtime.md) описывает границы и проверку.
