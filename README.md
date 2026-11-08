# mikai-cli

Неофіційний клієнт для [Mikai](https://mikai.me) — каталогу аніме з українським озвученням і субтитрами — у терміналі: TUI та CLI. Працює через [публічний API](https://api.mikai.me/public/docs).

Сайт: https://mikaicli.deitylamb.dev

## Встановлення

```sh
cargo install --git https://github.com/DeityLamb/mikai-cli
```

## TUI

```sh
mikai
```

Інтерфейс у стилі агента: внизу рядок команд, вище — історія всього, що ви робили (зберігається між запусками, гортається PgUp/колесом).

```
/search frieren        пошук, рядки нумеруються
/view frieren          сторінка перегляду: інфо, вибір озвучки й серії, mpv / браузер
/show #1               відкрити аніме — воно стає поточною сесією
/eps                   релізи по командах;  /eps #2 — серії команди
/watch 3               відкрити серію в браузері (команда запамʼятовується)
/play 3                те саме в mpv
/next                  наступна серія
/ep 3 · /status watching · /score 9
/schedule today · /updates · /list · /login
```

**Ctrl+P** — меню по центру: дії, живий пошук аніме, вибір команди та серії.

**Ctrl+H** — нещодавно переглянуті тайтли (локально + історія акаунта). Enter відкриває сторінку тайтлу з тією ж командою й фокусом на наступній серії (`/resume`).

Блоки історії згортаються: клік по заголовку, `Ctrl+↑/↓` + `Enter`, або `Ctrl+O` — згорнути всі минулі. `/help` — усе.

## CLI

```sh
mikai search frieren                 # пошук
mikai show frieren                   # картка (id, mal:52991, посилання або назва)
mikai eps frieren                    # релізи по командах
mikai eps frieren -t inari           # серії релізу
mikai open frieren -e 3 -t inari     # відкрити серію в браузері (--print — лише посилання)
mikai open frieren -e 3 --mpv        # у mpv
mikai schedule -d today              # розклад
mikai updates                        # новинки

mikai login                          # вхід за посиланням (підтвердити на mikai.me)
mikai login --with-key mk_…          # або готовим ключем з mikai.me/user/developers
mikai list -s watching               # мої списки
mikai list set frieren -s watching -e 5 --score 9
mikai list inc frieren               # +1 серія
mikai history | notifications | whoami

mikai api anime/1272/staff -q limit=5   # будь-який ендпоінт, сирий JSON
```

`--json` друкує сиру відповідь API для будь-якої команди. Ключ зберігається в `~/.config/mikai/config.json` (права 600), або передається через `MIKAI_API_KEY`.

Дані — [mikai.me](https://mikai.me). Відео належать провайдерам плеєрів. Проєкт не повʼязаний з командою Mikai.

## mpv

API дає лише сторінки плеєрів. У mpv відкриваються серії з плеєрів, які віддають пряме відео (зараз це ashdi, позначено ◆). Інші плеєри шифрують потік — їх mikai відкриває в браузері й нічого не обходить.
