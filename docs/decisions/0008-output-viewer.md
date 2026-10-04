# 0008. Поиск, исходные строки и Markdown в output viewer

Дата: 4 октября 2026. Статус: локальное решение реализации, не фиксированный продуктовый контракт.

## Сценарий и граница

Пользователь открывает README через Files, читает весь загруженный текст, ищет фразу и переходит к исходной строке без возврата в список. Просмотр принимает `:`; поиск и ввод строки сохраняют документ и позицию при отмене/ошибке. Успешный предметный command заменяет результат или выполняет Navigation/Workspace switch; предыдущий output не возвращается в новый контекст.

Определение Markdown по расширению `.md`/`.markdown` — логика Files. Общие переносы, source mapping, поиск и безопасное terminal presentation — Core (`src/viewer.rs`, `src/ui.rs`). Git возвращает Text. Независимо упакованный Catalog объявляет Markdown через тот же API и проверяется в двух Workspaces. Item остаётся объектом плагина, Block — результатом операции.

## Общий командный маршрут

`core.view.find`, `goto`, `next`, `previous`, `source` входят в CommandRegistry и возвращают Core-only `View(ViewRequest)`. Они не требуют Item или вызова plugin. UI интерпретирует request только при открытом output. Без аргумента find/goto открывают prompt, с аргументом выполняют операцию. `:120` нормализуется в `core.view.goto` с аргументом `120`; `/`, `g`, `n/N` и `v` вызывают те же registered commands. Команды доступны в общей палитре; обычный `:` доступен из просмотра и справки.

Поиск — case-sensitive literal substring в исходных строках, а не regex. Enter фиксирует запрос, пустой запрос очищает поиск. `n/N` переходят между совпавшими строками с wrap через границы документа; несколько совпадений внутри одной исходной строки пока считаются одним. Текущая строка выделяется целиком; статус показывает match ordinal/count. Если искомая фраза помещается в отдельной перенесённой строке, viewer показывает эту строку; поиск по скрытым Markdown-маркерам или фразе, пересекающей перенос, может показать начало исходной строки. No-match сохраняет позицию и сообщает результат.

`g` и `:120` используют 1-based source line. Номер вне документа, ноль или нечисловой ввод дают ошибку без закрытия. Переносы и Markdown не меняют исходную нумерацию. Прокрутка j/k/PgUp/PgDn работает по визуальным строкам; Home/End — по документу. При resize сохраняется исходная строка верхнего края, а не точная позиция внутри её переноса.

## Renderer и контракт

Source API 0.5 добавляет `Block.format: ContentFormat` (`Text` default, `Markdown`) и Core-only ViewRequest. Rust literals требуют format; JSON без format декодируется как Text. Host принимает manifest API 0.4 и 0.5; package/protocol остаются version 1. Старый плагин сохраняет обычный текст и прежний lifecycle, permissions и CommandInvocation. Плагинам запрещены View и WorkspaceChanged outcomes. Контракт документирован в `docs/plugin-api.md` и `docs/plugin-protocol.md`.

Renderer не принимает ANSI от плагина: управляющие символы нейтрализуются до layout. Цвет/стиль создаются Core из конечного набора semantic line kinds. Rendered rows содержат source index, безопасный текст и kind. Rich layout показывает source number только у первой визуальной строки; gutter продолжений пустой. Компактная компоновка сохраняет source mapping и команды без обязательной колонки номеров. Повторное уведомление о том же размере перед кадром не пересчитывает позицию; source anchor применяется только при реальном resize, чтобы j/k/PgDown проходили сквозь длинный перенесённый абзац.

Раскладка кэшируется для текущего Block, ширины панели и presentation/source режима; новый результат инвалидирует кэш. Кадр копирует только видимые строки. Это избегает повторного Markdown parsing и переноса всего документа при каждом вводимом символе.

Markdown — небольшой line-oriented renderer без нового crate: ATX headings, простые bullet lists, blockquotes, fenced code, horizontal rules; inline emphasis/code markers упрощаются, inline links показывают label и URL. Heading/code/quote имеют разные terminal styles; код внутри fence не преобразуется как inline Markdown. `v` сохраняет source position и переключает оформленный текст/исходник. Это не полная реализация CommonMark/GFM: сложные вложенные структуры, HTML, изображения и отдельный табличный layout не добавлены. Неподдерживаемая разметка остаётся читаемым текстом. Unicode width учитывается при переносе; текст ограничивается реальной панелью без autowrap терминала.

Files preview загружает целый файл до 128 KiB; более крупный файл даёт явную ошибку, не скрытое обрезание. Предел оставляет место для JSON escaping в 1 MiB protocol response. Paging больших файлов с lazy loading остаётся отдельной задачей.

## Проверка

UI tests: русский поиск, n/N и wrap, отмена/пустой ввод/no-match, `g`/`:120`/адресуемая goto, неверный номер, компактный/rich layout, resize, long paragraph и поиск в его конце, code literals, source toggle, terminal control sanitization. Runtime tests проверяют полный preview за старым пределом 8 KiB, worst-case JSON escaping, oversized-file error/recovery и совместимость manifests/Blocks 0.4. Catalog package проходит тот же renderer, поиск и source-line command в двух Workspaces через обычный loader. Unix PTY проходит реальную клавиатуру, Markdown/source, поиск и goto. Performance-цели остаются отдельными benchmark задачами; smoke timings не подтверждают их достижение.

Debug PTY smoke, macOS, документ 2000 строк: до кэширования поиск 125.1 ms, goto 123.3 ms; после — 10.8 ms и 10.0 ms. Первый кадр после изменения 1243.6 ms (выше startup-цели <300 ms), Files directory/parent по 2.6 ms. Это единичные замеры, не release benchmark.
