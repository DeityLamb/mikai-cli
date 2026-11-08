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

Головний екран — щоб знайти тайтл, сторінка тайтлу — щоб дивитись.

```
/search frieren        пошук, рядки нумеруються
/view #1               сторінка тайтлу (або /view frieren)
/random                випадковий тайтл
/schedule today        розклад
/updates               новинки
/list watching         мої списки
/clear [recent|all]     стерти історію; recent — нещодавні, all — усе
/login · /logout · /help · /quit
```

**Сторінка тайтлу:** опис, вибір команди (`c`: озвучення/субтитри → команда) і плеєра (`p`), сітка серій, `⏎` дивитись (mpv або браузер), `w` переглянуто, `s` статус у списку, `o` mikai.me. Відкривається з тією ж командою й на наступній серії, якщо ви вже дивились.

**Ctrl+P** — живий пошук аніме. **Ctrl+Shift+P** — меню (або `>` у пошуку, якщо термінал не розрізняє Shift). **Ctrl+H** — нещодавно переглянуті.

Усе, що ви робите, лишається в історії (гортається PgUp/колесом, блоки згортаються `Ctrl+↑`/`Ctrl+O`) і зберігається між запусками.

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
