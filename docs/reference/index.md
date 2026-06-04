# CLI Reference

## Synopsis

```
l0-cache [OPTIONS] [COMMAND]...
```

## Options

### Execution Modes

#### `--raw`
Run the command but print full output without truncation. Metrics are
still logged. ANSI stripping is still applied.

#### `-i`, `--interactive`
Force passthrough mode. stdin, stdout, and stderr are inherited by the
child process. No capture, no filtering, no metrics.

### Filtering

#### `--head <N>`
Number of lines to keep from the start of output. Default: 30.

#### `--tail <N>`
Number of lines to keep from the end of output. Default: 30.

#### `--tail-error <N>`
Number of tail lines to keep when the child exits with non-zero status.
Default: 120.

#### `--threshold <N>`
Minimum number of output lines before truncation is applied. If the
total output is below this threshold, it is printed in full. Default: 100.

### Adaptive Tuning

Adaptive parameter auto-tuning is enabled by default.

#### `--no-auto`
Disable adaptive auto-tuning of parameters.

#### `--auto`
Enable adaptive auto-tuning (redundant as it is now enabled by default, but supported for backward compatibility).

#### `--auto-floor <N>`
Floor limit for success optimization decay. Default: 10.

#### `--auto-ceiling <N>`
Ceiling limit for failure backoff tail expansion. Default: 1000.

### Metrics

#### `--stats`
Print an aggregated token savings report and exit. Does not run a command.

#### `--since <DURATION>`
Filter the stats report to entries within the given time window.
Requires `--stats`. Format: `Nd`, `Nh`, `Nm`, `Ns` (e.g. `7d`, `24h`).

#### `--token-factor <N>`
Divisor used for token estimation. The number of bytes is divided by this factor to estimate the token count. Default: 4.

### Utility

#### `--doctor`
Diagnose system installation, PATH resolution, shell configuration, and active LLM editors. Prints a SOTA terminal health report.

#### `--completions <SHELL>`
Generate shell completion script and print to stdout. Valid values:
`bash`, `zsh`, `fish`, `elvish`, `powershell`.

#### `--version`, `-V`
Print version with git commit hash (e.g. `l0-cache 0.1.0 (abc1234)`).

#### `--help`, `-h`
Print help message.
