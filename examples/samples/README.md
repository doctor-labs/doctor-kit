# Face and theme samples

All nine captures come from `example-doctor` over `examples/fixture` (8 findings, score 57/100).
`<face>-<theme>.txt` is the raw output with ANSI codes (view with `cat` or `less -R`);
`<face>-<theme>.plain.txt` is the same text with the codes stripped.
Regenerate with `examples/samples/gen.sh`.

Run any variant from `examples/`:

    example-doctor scan ./fixture --face rich --theme clinical --color always

## Faces (`--face`)

- `plain`: header line, one line per finding (severity, rule, location, message), then severity counts. Minimalist; closest to the headless text.
- `rich`: boxed report. Score gauge, severity counts, findings grouped by category, each with its fix. Default.
- `compact`: one summary line (tool, score, counts) then an aligned findings table. Good for terminals with little room.

Also: `--json` (doctor/1 envelope), `--score` (number only), `--headless` (plain, no color).

## Themes (`--theme`)

- `mono`: no hue; bold, underline and dim only. Safe on any terminal and in logs.
- `clinical`: green / amber / red, cyan for info. Default for the example.
- `contrast`: bright, bold colors for dark backgrounds and low vision.

A `theme.toml` next to the target (or `--theme-file`) overrides any token
(`ok info low medium high critical heading dim accent`), for example
`base = "clinical"` then `high = "bold #ff5500"`.
