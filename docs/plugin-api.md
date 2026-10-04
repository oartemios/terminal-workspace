# Plugin API v0.3 — draft

Версия исходного Rust-контракта обозначена `PLUGIN_API_VERSION = "0.3"`. Это ранний API, связанный с Core при сборке, без стабильного ABI и без sandbox. Внешний пример находится в `examples/custom-plugin`; Files использует те же публичные контракты.

## Данные и вызовы

- `Plugin::items` задаёт корневые Items стабильной Group. `Plugin::view` опционально расширяет загрузку вложенными контекстами; default поддерживает только корень.
- `GroupView` возвращает Items, заголовок, opaque `location`, опциональный родительский `CommandInvocation` и `command_defaults` для команд в текущем контексте.
- Необязательный `Plugin::item_icon(&Item) -> char` задаёт символ типа для списка. Default — •; Core допускает только безопасный символ шириной в одну ячейку, иначе использует default. Нативный `kind` сохраняется; существующие плагины не требуют изменений.
- `Item.id` и `Item.kind` имеют нативный смысл внутри плагина. Core не интерпретирует их как пути или фиксированную классификацию объектов.
- `Plugin::actions` задаёт допустимые действия объекта. Максимум одно Action может иметь `is_default = true`: Enter вызывает его. `a` показывает весь список. Если default отсутствует, Enter открывает список действий.
- `Plugin::execute` возвращает `CommandOutcome::Output(Block)` для структурированного результата или `Navigate(Navigation)` для смены контекста группы. Navigation задаёт группу, opaque location и необязательный ItemId, который следует выделить после перехода.

## Контекст выполнения

App проверяет регистрацию команды, состояние активации и выданные разрешения. TUI формирует одинаковые вызовы из действий, палитры, `:commands` и bindings. Явный аргумент командной строки имеет приоритет; затем используются `command_defaults` текущего представления, затем выбранный Item для команды его плагина, требующей Item.

Core загружает новый GroupView до изменения отображаемого контекста. Ошибка оставляет текущий список и показывается в statusline. Для каждой локации сохраняются фильтр, сортировка и выбранный Item; возврат восстанавливает их. Если явно запрошенное выделение скрыто фильтром, фильтр очищается, чтобы показать объект.

Workspace передаётся при каждом вызове. Плагин не должен привязывать данные или состояние к одному проекту. Проверки разрешений являются моделью доступа в текущем процессе, а не защитой от прямого обращения стороннего Rust-кода к ОС.

## Files

Files сам нормализует пути, проверяет их принадлежность Workspace, задаёт default Action для каталогов и родительский вызов. `files.open` и `files.parent` возвращают Navigation; `files.preview` и `files.path` возвращают Blocks. Внутренние symlinks сохраняют логическую локацию, ссылки за пределы Workspace не открываются.

Изменения относительно первой заготовки: `execute` возвращает `CommandOutcome` вместо Block, Action получает `is_default`, а Item хранит нативный `kind` вместо Files-специфичного `is_directory`.

## Core-команды и конфигурация (v0.2)

`CommandInvocation.args: Vec<String>` передаёт аргументы без выбранного доменного Item. В существующие литералы вызовов добавь `args: Vec::new()`. `App::invoke` теперь требует `&mut self`, поскольку registry выполняет также Core-команды, изменяющие активацию и разрешения. `App::disable` и `disable_for_session` возвращают `Result<(), String>`; неизвестные plugin id отклоняются. Namespace `core` зарезервирован, plugin commands по-прежнему принадлежат namespace их плагина.

`App::open` читает и сохраняет конфигурацию проекта; `App::new` создаёт сессию без записи на диск. `Workspace::plugin_settings(id)` возвращает необязательный JSON-объект, который интерпретирует плагин; `Workspace::overrides` предоставляет сохранённые overrides. `App::configuration_error` сообщает ошибку чтения. `grant_default` применяет явный default приложения только без сохранённого решения о разрешениях; обычный `grant` — явная выдача, `revoke` — сохраняемый отзыв.

Core-команды `core.plugins`, `core.plugin.enable/disable/suspend`, `core.permissions`, `core.permission.grant-read/revoke-read` используют тот же registry, что Files и custom plugin. Все, кроме `core.plugins`, принимают один plugin id в args. В TUI palette/binding/Action цель задаёт выбранная вкладка плагина; Item для них не нужен. После отключения или отзыва UI очищает данные текущего представления; плагин и его settings остаются установлены и доступны для включения.

## Переключение Workspace и bindings (v0.3)

`Plugin::keybindings` имеет default `Vec::new()`. `KeyBinding { keys, command_id, scope }` объявляет локальную последовательность; `BindingScope::Plugin(String)`, `Group { plugin, group }` и `View { plugin, group, location }` задают область её действия. Идентификаторы group/location остаются opaque. Плагин может объявлять только свои зарегистрированные CommandId и свой namespace. Files и внешне упакованный Catalog используют один контракт.

App предоставляет `keybindings(plugin, group, location)` и `binding_diagnostics()`. Global bindings принадлежат Core. Project-local overrides имеют приоритет перед defaults; неоднозначные последовательности отключаются с диагностикой. Поддерживаются только печатные символы без пробелов; зарезервированные клавиши, JSON-схема и правила приоритета описаны в [решении реализации](decisions/0004-workspace-switching-and-bindings.md).

`core.workspace.open` принимает один путь в args, `core.workspace.previous` — ни одного. `App::switch_workspace` загружает новый проект до замены текущего, сохраняя установленные реализации и registry. Относительный путь считается от текущего корня. Разрешения и settings не переносятся; suspend отдельно сохраняется для каждого проекта в пределах сессии. Defaults из `install(..., enabled)` и явно вызванного `grant_default` применяются к новому проекту только при отсутствии сохранённого решения.

`CommandOutcome::WorkspaceChanged` — новый вариант enum, возвращаемый Core после успешного переключения. Обнови exhaustive matches на CommandOutcome. Это смена контекста, не доменный Item и не Block. UI снимает старые результаты и загружает представления через тот же Plugin API с новым Workspace. Плагин получает Workspace при каждом вызове и не должен хранить project-specific данные без разделения по контексту. Lifecycle hooks и runtime isolation остаются вне текущего API.

Defaults не являются фиксированным контрактом Plugin API. [Итерация 2.1](decisions/0005-keyboard-defaults.md) оставляет Files `p` → `files.preview`, Catalog — `p` → `catalog.read` в View Introduction. Открытие/default Action и parent invocation вызываются общей навигацией Core; публичный API v0.3 и прежние CommandId не меняются. Удалённые defaults можно явно восстановить через project-local overrides.
