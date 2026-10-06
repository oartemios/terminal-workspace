# Executable plugin protocol v1

Package format 1 и source Plugin API 0.7. Реализация: `src/runtime/`; ограничения/trust/enforcement описаны в [решении runtime](decisions/0006-executable-plugin-runtime.md).

## Пакет

`plugin.json` содержит:

```json
{
  "package_version": 1,
  "protocol_version": 1,
  "api_version": "0.7",
  "executable": "plugin",
  "args": [],
  "environment": [],
  "credentials": [],
  "plugin": {
    "id": "catalog",
    "name": "Catalog",
    "groups": [{"id": "sections", "title": "Sections"}],
    "commands": [{"id": "catalog.read", "title": "Read note", "requires_item": true}],
    "permissions": [],
    "keybindings": [],
    "refresh": [["sections", "OnFocus"]]
  }
}
```

Это минимальный schema example, не полный manifest примера Catalog. SDK `write_package` генерирует полный descriptor из `Plugin` и копирует executable. ID пакета — ASCII alphanumeric/`-`/`_`, максимум 64 символа, кроме `core`; directory name должен совпадать. CommandId принадлежит `id.` namespace. Groups/commands должны иметь уникальные id; bindings — собственные команды и объявленные groups. Максимум 256 groups, 512 commands, 1024 bindings. Manifest ≤ 1 MiB; executable — один обычный файл с basename внутри пакета, без symlink и совпадения с `plugin.json`. Installation выставляет mode 0755; launch выполняется без shell expansion.

`environment`/`credentials` — списки имён host environment variables, не значений; непустые списки требуют соответствующей declared permission. Host не передаёт остальной environment. `args`, `environment`, `credentials`, `permissions`, `keybindings` могут быть пустыми; описанный пример показывает их явно.

## Транспорт

UTF-8 JSON, один объект и newline на сообщение. Stdout — только protocol replies; stderr подавляется host. Requests исполняются последовательно, unsolicited events не поддерживаются. Host начинает request id с 1 и увеличивает его в каждом worker. Reply обязан повторять id и protocol 1.

Handshake:

```json
{"protocol":1,"request_id":1,"op":"describe"}
```

```json
{"protocol":1,"request_id":1,"result":{"Ok":{"id":"catalog","name":"Catalog","groups":[],"commands":[],"permissions":[],"keybindings":[]}}}
```

Показанный ответ иллюстрирует форму: в реальном запуске descriptor должен точно совпадать с `manifest.plugin`, включая порядок массивов. Совместимость версии и namespace проверяются до запуска, descriptor — после trust/start. Ошибка:

```json
{"protocol":1,"request_id":2,"result":{"Err":"Unknown section"}}
```

Messages ограничены 1 MiB. Startup describe: deadline 5 s, обычный request: 1 s. SDK отвечает небольшим `Err`, если его результат превысил лимит; raw malformed/oversized output приводит к disconnect/termination. Reply `Err` обычной операции сохраняет соединение. Исключение/panic SDK может завершить worker после error reply. Restart создаёт новый process и новую последовательность id.

## Контекст

Все предметные requests содержат:

```json
{"root":"/absolute/canonical/project","settings":{"greeting":"Hello"},"permissions":[]}
```

Это `context`, settings только текущего plugin. Root и Item/group/location identifiers не превращаются в глобальную модель данных. Для path helpers SDK reconstructs `Workspace` с granted permissions. Не пересылаются config других plugins, credentials values как JSON или UI overrides. Environment forwarding происходит отдельно при process spawn. Переключение Workspace завершает старый worker; requests нового проекта не применяются к старому.

## Операции

`view`:

```json
{"protocol":1,"request_id":2,"op":"view","context":{"root":"/project","settings":{},"permissions":[]},"group":"sections","location":"intro"}
```

Успех `result.Ok`:

```json
{"view":{"title":"Introduction","location":"intro","items":[{"id":"intro.note","title":"Welcome note","kind":"note"}],"parent":{"id":"catalog.parent","item":null,"args":[]},"command_defaults":[]},"icons":{"intro.note":"•"}}
```

Items сохраняют нативный смысл `kind`; id уникальны в view. Parent и command defaults принадлежат registered commands данного plugin. Icons — optional entries в обязательном объекте `icons`; небезопасный/неодноклеточный символ заменяется Core на fallback.

`refresh` принимает тот же context/group/location, вызывает `Plugin::refresh` и возвращает тот же `ViewReply`, включая icons. Плагины со стратегией refresh должны быстро отдавать plugin-owned cache из `view`, не ждать сеть. Ошибка refresh оставляет ранее показанный view в UI со stale-индикатором.

`actions` получает `context` и `item` с теми же полями id/title/kind. Успех — массив:

```json
[{"label":"Read note","command_id":"catalog.read","invocation":{"id":"catalog.read","item":"intro.note","args":[]},"is_default":false}]
```

Core сверяет action command_id/invocation.id и ownership; максимум один default Action. Item для request берётся из повторно проверенного текущего view. Это защищает основной command route от Action, подменяющего его Core-командой.

`execute` получает `context` и `invocation`. Пример:

```json
{"protocol":1,"request_id":3,"op":"execute","context":{"root":"/project","settings":{},"permissions":[]},"invocation":{"id":"catalog.read","item":"intro.note","args":[]}}
```

`result.Ok` имеет один из вариантов:

```json
{"Output":{"source":"Catalog","status":"ok","content":"Hello"}}
```

```json
{"Navigate":{"group":"sections","location":"intro","selected":null}}
```

Output — Block результата команды. Navigation — контекстная смена представления собственного plugin. `WorkspaceChanged` зарезервирован для Core и отклоняется от plugin. Args независимы от выбранного Item; requires_item проверяет host. Одна registered операция использует тот же invocation из bindings, command line, palette и Actions.

`BindingScope` сериализуется как Rust serde externally tagged enum: `{"Plugin":"files"}`, `{"Group":{"plugin":"catalog","group":"sections"}}`, `{"View":{"plugin":"catalog","group":"sections","location":"intro"}}`; `"Global"` допустим только для Core. KeyBinding содержит `keys`, `command_id`, `scope`. Reserved keys, scope priority и project-local overrides описаны в [решении ввода](decisions/0004-workspace-switching-and-bindings.md); актуальные defaults — в [итерации 2.1](decisions/0005-keyboard-defaults.md).

## Rust SDK и lifecycle

Author implements `Plugin`, затем worker entrypoint вызывает `runtime::serve_plugin(&plugin)`. `runtime::write_package(&plugin, executable, args, destination)` создаёт пакет. Files и `examples/custom-plugin` используют эти же функции. Rust ABI между executable и host не разделяется.

API 0.4 добавляет serde DTO, категории permissions, `start(workspace, permissions)`, `stop`, `try_actions`, `runtime_status`, `installation_present`. Defaults совместимы с простыми linked implementations; id/name могут возвращать borrowed `&str`. Hooks start должны быть идемпотентными: Core/SDK могут вызвать их перед предметной операцией. `try_actions` позволяет вернуть диагностируемую ошибку вместо panic. Runtime-specific status не обязывает плагин иметь доменную connection.

Host polling для ProcessPlugin запускает worker и handshake вне TUI-потока; synchronous SDK path сохраняется; SDK вызывает native plugin start перед view/actions/execute. Stop процесса — жёсткое завершение группы; native cleanup hooks не гарантированы. Для linked доверенного SDK usage Core вызывает stop hook, но не может уничтожить произвольные native threads. Внешний контракт остаётся draft, capabilities optional, events/refresh strategies появятся отдельно.

API 0.5: Output Block может содержать `"format":"Text"` или `"format":"Markdown"`; отсутствие поля в API 0.4 ответах означает Text. Content содержит исходный текст без ANSI. Host 0.5 принимает manifests 0.4 и 0.5. Плагины не возвращают Core-only `View` outcomes; это механизм маршрутизации команд viewer внутри host.

API 0.6: неблокирующий host scheduling использует те же describe/view/actions/execute messages и deadlines; wire protocol остаётся 1. API 0.7 добавляет refresh operation без изменения framing. Host принимает manifests API 0.4–0.7. Один executable worker обслуживает последовательные requests; completion и cancellation реализованы host, unsolicited replies или native threads в plugin не требуются. Startup/handshake и начальная view загрузка также идут в фоне. Details: [polling contract](plugin-api.md#api-06-polling-фоновой-работы), [решение 0010](decisions/0010-background-plugin-runtime.md).
