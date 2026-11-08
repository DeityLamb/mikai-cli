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

Вкладки «Пошук», «Розклад», «Новинки», «Мій список». `Enter` — відкрити, `Esc` — назад, `?` — усі клавіші.
У серіях `Enter` відкриває плеєр у браузері, `p` перемикає плеєр, `w` позначає переглянутим.

## CLI

```sh
mikai search frieren                 # пошук
mikai show frieren                   # картка (id, mal:52991, посилання або назва)
mikai eps frieren                    # релізи по командах
mikai eps frieren -t inari           # серії релізу
mikai open frieren -e 3 -t inari     # відкрити серію в браузері (--print — лише посилання)
mikai schedule -d today              # розклад
mikai updates                        # новинки

mikai login                          # вхід за посиланням
mikai login --with-key mk_…          # або готовим ключем з mikai.me/user/developers
mikai list -s watching               # мої списки
mikai list set frieren -s watching -e 5 --score 9
mikai list inc frieren               # +1 серія
mikai history | notifications | whoami

mikai api anime/1272/staff -q limit=5   # будь-який ендпоінт, сирий JSON
```

`--json` друкує сиру відповідь API для будь-якої команди. Ключ зберігається в `~/.config/mikai/config.json` (права 600), або передається через `MIKAI_API_KEY`.

Дані — [mikai.me](https://mikai.me). Відео належать провайдерам плеєрів. Проєкт не повʼязаний з командою Mikai.
