# Глобальные пакеты и процессный runtime

Статус: локальное решение реализации этапа 3, не исходное требование о конкретном формате, ABI, runtime или layout.

## Выбор

Первый внешний контракт — JSON Lines по stdin/stdout отдельного локального executable. Пакет состоит из `plugin.json` и одного самостоятельного executable; дополнительные ресурсы, архивы, зависимости и обновление на месте пока не поддерживаются. Поддерживаются macOS/Linux. Используются стандартные процессы Rust, Unix poll/process groups и уже выбранный `serde_json`; для публичных DTO добавлен `serde = 1.0.197` с derive, доступный в offline-кэше и проверенный с Rust 1.74.1. Async runtime и динамический Rust ABI не вводятся.

Версии разделены: package format 1, wire protocol 1, source Plugin API 0.4. На этом раннем этапе API проверяется на точное совпадение. Нет обещания стабильного ABI или совместимости произвольных будущих версий. Подробный контракт — в [описании протокола](../plugin-protocol.md).

## Установка и discovery

Глобальный store по умолчанию: `~/.local/share/terminal-workspace/plugins`; абсолютный `TW_PLUGIN_DIR` переопределяет путь. Store не принадлежит конкретному Workspace. Source package выбирается явно, установка копирует manifest и executable через соседний временный каталог и rename. Не копируются source trust markers или другие файлы. Manifest и executable должны быть обычными файлами, имена/namespace/команды/bindings проверяются. Symlinks в этих точках отклоняются; filesystem checks не являются полной защитой от гонок другого процесса того же пользователя. Полная durability при отключении питания и межпроцессная блокировка не реализованы.

Discovery читает только manifests, не исполняя код. Groups, Commands, bindings и запрошенные permissions объявлены в manifest. Новый пакет по умолчанию не активируется и не получает разрешения; ранее сохранённая project-local активация при reinstall сохраняет смысл. Несовместимый или повреждённый пакет даёт понятную недоступность/диагностику; остальные плагины продолжают работать. Отсутствующий executable отличается от отключения.

Files пакуется из дистрибутива при первом запуске нового store как явно разрешённый application default. Он запускается через тот же `ProcessPlugin` и `serve_plugin`, что independently built Catalog. Существующий пакет с id `files` не получает автоматического доверия. Маркер первого запуска не позволяет bootstrap отменять explicit uninstall. Повторную установку пользователь выполняет явно. Нет скрытого привилегированного исполнения Files в production TUI.

## Доверие, активация, availability и connection

Установленный custom package остаётся untrusted. `core.plugin.trust` или CLI `tw plugins trust` — явное решение разрешить локальное native execution. Доверие хранится отдельно от project-local activation и permissions. Trust marker не является подписью, checksum или защитой от изменения файлов тем же OS-пользователем. Для изменения пакета предусмотрен uninstall/install с новым trust; hot upgrade и file watching не реализованы.

Workspace/session activation сохраняют смысл предыдущих этапов. API состояния отдельно показывает installation presence, Workspace enabled, session enabled, Availability (`Available/Untrusted/Incompatible/Missing/Failed`) и Connection (`Disconnected/Running/Failed/InProcess`). `InProcess` относится к доверенным linked fixtures/SDK usage; TUI загружает first-party и custom packages процессным путём. Core `core.plugins` выводит поля короткими строками, доступными в компактном терминале; malformed discoveries перечисляются отдельно. Начальный выбор предпочитает активный доступный плагин, а сохранённый выбор проекта восстанавливается.

`core.plugin.untrust`, suspend, permanent disable, permission revoke, Workspace switch и Drop App прекращают процессную активность текущего host. Uninstall — отдельная глобальная операция; project-local конфигурация не удаляется, trust не переносится в новую установку. Удаление другим host обнаруживается через состояние/discovery; это не мгновенное межпроцессное уведомление. Одновременное управление одним store несколькими hosts не координируется.

## Lifecycle и ошибки

Процесс запускается лениво после проверки активации, доверия и всех declared permissions. Он получает pipes, закрытый stderr и рабочий каталог собственного пакета. Не получает terminal stdin/stdout. Handshake должен точно совпасть с установленным descriptor. Workspace context передаётся общим API: канонический root, settings только данного плагина и granted permissions; чужие settings и overrides не пересылаются.

Ответы ограничены 1 MiB и проверяются по protocol/request id. Неблокирующие pipes не допускают бесконечного write/read; ожидание handshake ограничено 5 s, обычного ответа — 1 s. Более длинный startup budget нужен для холодного запуска executable/interpreter; он не подтверждает performance-цели. Сбой процесса, неверный JSON/id, oversized response или timeout прекращают worker и оставляют host/другие плагины доступными. Обычная доменная ошибка — ответ `Err` без обязательного disconnect. `core.plugin.restart` или повторный enable позволяют явно повторить запуск.

Worker создаёт отдельную process group. Stop использует SIGKILL группы и reap непосредственного дочернего процесса; обычные дочерние процессы тоже останавливаются. Это жёсткое завершение: произвольные cleanup hooks не гарантируются, внешние изменения/файлы не откатываются. Злонамеренный процесс может создавать отделённые группы; их удержание и OS sandbox не реализованы. Stdout зарезервирован под протокол; печать произвольного текста нарушает контракт. View icons передаются вместе с view, чтобы не делать отдельный RPC на каждый Item.

Вызовы пока синхронные с ограниченным временем ожидания: зависшая команда временно задерживает обработку ввода до timeout. Асинхронная загрузка, отмена фоновых tasks и неблокирующее сетевое обновление относятся к этапу 5. Нет утверждения, что медленная сеть уже поддерживается без задержек TUI.

## Permission model: что проверяется

В конфигурации JSON version 1 сохраняются известные categories `WorkspaceRead`, `WorkspaceWrite`, `Process`, `Network`, `Credentials`, `Environment`; неизвестные имена сохраняются, но не выдаются. Каждый declared permission обязателен для старта/предметных вызовов данного плагина; optional capabilities внутри одного запуска пока не моделируются. Discovery и listing Groups читают manifest и не требуют запуска.

| Категория | Проверяемая граница текущей реализации |
| --- | --- |
| WorkspaceRead | Host gate до запуска/вызова; SDK `Workspace::read_path` проверяет grant и принадлежность root |
| WorkspaceWrite | Host gate; SDK `write_path` проверяет grant, root и parent для нового файла, отклоняет escape/dangling symlink |
| Process | Host gate для плагина, объявившего предметное использование процессов; инфраструктурный запуск worker отдельно разрешается trust |
| Network | Host gate для объявленной сетевой capability; kernel socket restrictions/domain allowlist отсутствуют |
| Credentials | Host gate; только явно перечисленные `manifest.credentials` environment names передаются worker после grant |
| Environment | Host gate; `env_clear` и передача только явно перечисленных `manifest.environment` names после grant |

Фактически enforced: отсутствие исполнения untrusted package через host; отказ вызова при недостающем declared grant; namespace/CommandRegistry checks; минимизированный передаваемый context/environment; проверки SDK path helpers; ограничения протокола и stop процесса/группы. Actions и view defaults могут ссылаться только на команды собственного плагина; plugin не может возвращать Core WorkspaceChanged.

**Это не OS sandbox.** Доверенный native executable имеет права OS-пользователя и может обходить SDK, читать/писать файлы, создавать процессы, обращаться к сети и искать credentials самостоятельно, даже если не объявил capability. Path helpers не устраняют TOCTOU между проверкой пути и использованием. Environment allowlist не является классификатором секретов: пользователь должен считать каждое передаваемое имя явным раскрытием его значения. Credentials provider здесь — named environment forwarding, а не vault/keychain и не полноценная авторизация GitHub. Core не записывает их значения в Workspace config или manifest. Нельзя считать непроверенный пакет безопасным после grant узкой capability.

## Сквозные проверки и ограничения

Rust integration tests устанавливают пакеты в отдельный временный global store, отличающийся от двух project roots. Проверяются installation/trust/activation/permissions, restart и reinstall, API mismatch/missing/malformed packages, handshake mismatch, crash, hang, bad JSON/id, oversized response, domain errors, чужой Core Action, worker/child termination, environment/credentials forwarding, отсутствие чужих settings и SDK read/write escapes.

Отдельный Catalog package собирается собственным executable и устанавливается через UI palette. Тот же пакет работает с двумя Workspaces и settings; binding, colon, palette и Action используют текущий контекст. Files использует тот же внешний loader. PTY проверяет package CLI, uninstall/bootstrap separation, установку из палитры, trust, session activation, статус процесса, Unicode/resize и восстановление терминала. Tests не пишут в реальный пользовательский global store.

PTY smoke check от 4 октября 2026, macOS, debug build, новый временный global store с первичным copy Files и сохранением defaults: первый кадр 1060.7 ms, открытие каталога 4.0 ms, возврат 4.1 ms. Единичный замер не заменяет release benchmark; startup выше цели < 300 ms. Performance-цели остаются задачей этапа 7.
