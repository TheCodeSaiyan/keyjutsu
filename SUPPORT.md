# Getting help

- **Something does not work:** run `keyjutsu doctor` first. It starts each
  shell once in a throwaway terminal and says what worked, which answers most
  "the performance does nothing" questions. Then open an issue with its
  output (`keyjutsu doctor --json` has everything). The report contains your
  Windows build, shell versions and paths, and your Windows Terminal font and
  colour scheme; it does not contain your profile scripts or history.
- **A performance stalls with the shell's own prompt showing:** your
  PowerShell profile is probably replacing the prompt after KeyJutsu's
  wrapper. Try the clean profile (`--clean`, or Profile: Clean in the app).
- **A security problem:** see [SECURITY.md](SECURITY.md), not the issue
  tracker.
- **A question about how it works:** start with
  [the architecture overview](docs/architecture/overview.md).

KeyJutsu is early and maintained on a best-effort basis. There is no paid
support.
