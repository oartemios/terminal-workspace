# 0010. Неблокирующие операции executable plugins

Дата: 5 октября 2026. Статус: локальное решение реализации для #4.

Сценарий: медленный startup/handshake, начальная загрузка view, Actions или команда одного плагина не задерживает клавиатуру и локальный Files. При смене Workspace или lifecycle cancellation старая работа прекращается и не заменяет новый контекст. Предметная логика остаётся в плагине; scheduling, отмена, status и проверка response принадлежат Core/runtime.

## Решение

Добавить optional polling-контракт source API 0.6 (`BackgroundRequest/Response`, `Plugin::poll_background`, `App::poll_background/poll_invoke`) поверх неизменного executable protocol 1. Один CommandRegistry и единые проверки регистрации, permissions, View/Actions/Outcome обслуживают все способы вызова; UI обрабатывает response тем же способом, что прежний synchronous route.

Для ProcessPlugin использовать `std::thread` и bounded `sync_channel(1)`, без нового crate или async runtime. На плагин хранится одна pending операция, один completion и один живой executable worker. Поток на время операции владеет Worker/pipe state; по завершении возвращает его host, чтобы следующая операция переиспользовала процесс. Такой выбор локален для текущего последовательного JSON-lines transport и не фиксирует будущую модель runtime.

Новая операция ждёт занятого worker без очереди; совпадающие polls не исполняют request повторно. Completion сопоставляется с request и Context (root/settings/permissions). Stop удаляет pending и completed state. У каждой generation свой cancellation handle и receiver, поэтому старый sender не может вернуть состояние в новую generation. UI проверяет root и epoch; переход в другой plugin/group, смена Workspace или новый режим подавляют устаревший UI result. Обновление списка сохраняет выбранный ID, когда он остаётся в данных.

Cancellation handle регистрирует pid под mutex. Он завершает process group через SIGKILL, включая детей; Worker reaps child и снимает регистрацию под тем же mutex, чтобы поздняя отмена не сигнализировала переиспользованный PID. Suspend, Workspace disable, revoke permission/trust, switch, uninstall и shutdown используют общий stop. Поток без process проверяет cancelled перед spawn и после регистрации свежего pid; mutex не удерживается во время OS startup, поэтому отмена не ждёт spawn. Core не ждёт handshake/request deadline при cancellation; detached поток освобождается после закрытия убитых pipes. Это process containment, не OS sandbox и не rollback внешних изменений.

`Connection::Loading` и statusline сообщают о pending work. Основной loop вызывает `Ui::tick`; ожидание первого input byte уменьшено со 100 до 16 ms, чтобы idle completion чаще попадал в render. Это настройка polling, не доказательство достижения performance целей. Декодирование Esc sequence сохраняет прежние короткие timeout.

## Совместимость и ограничения

Source API — 0.6, package/protocol — 1. Host принимает manifests 0.4/0.5/0.6; старые executable plugins автоматически получают background scheduling через adapter. Independently packaged Catalog использует тот же путь без privileged registration или собственной async реализации. Linked trusted implementations имеют synchronous default; host не обещает отмену произвольных native threads. Synchronous App methods остаются доступными для короткого SDK usage; при pending polling работе смешивать эти маршруты нельзя.

Обычный request сохраняет deadline 1 s, describe — 5 s; сетевые timeout и refresh стратегии могут потребовать последующей итерации. Files bootstrap/discovery, сохранение конфигурации и другие короткие Core I/O операции не переведены в фон. Это не cache/refresh контракт #5 и не event bus #9. При уходе из контекста результат команды может быть скрыт, но процесс продолжает работу до completion или lifecycle stop; внешняя операция не откатывается.

## Проверка

- Gated protocol peer задерживает describe, view, actions и execute до явного release; Files и polling/status остаются доступными.
- Отдельные tests проверяют все lifecycle границы во время startup и initial view, прекращение детей reused worker и новый process/context после Workspace switch.
- Timeout handshake/view/command, crash и malformed response локализованы; polling restart возвращает работоспособный worker.
- UI tests проверяют уход из gated view, позднюю команду после Workspace switch и сохранение initial load после команды без необходимого Item.
- Git и independently packaged Catalog проверяют все четыре командных маршрута с ожиданием polling completion; Catalog работает в двух Workspaces через тот же package API.
- Полный Rust/custom-plugin suite, fmt, clippy и Unix PTY проверяются перед PR. Release performance benchmarks и Linux/macOS CI matrix остаются отдельными доказательствами; локальные timings не подтверждают MVP targets.
