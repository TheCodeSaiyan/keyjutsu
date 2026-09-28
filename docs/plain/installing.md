# Installing KeyJutsu

**By the end of this page:** KeyJutsu is on your computer, and it has checked
that your computer is ready.

## Before you start

You'll need the KeyJutsu installer. Its name is
`KeyJutsu_0.1.7_x64-setup.exe`, and it's on the
[releases page](https://github.com/TheCodeSaiyan/keyjutsu/releases/latest). If
that page is empty, no copy has been published yet, and someone who builds
software has to make one for you.

You'll also need to know your computer's **administrator** password, or have
someone who does nearby. You type it once.

## Steps

### 1. Start the installer

Double-click `KeyJutsu_0.1.7_x64-setup.exe`.

### 2. Get past the warning

KeyJutsu's installer is signed: it carries a stamp that says who made it,
TheCodeSaiyan Ltd, which Windows checks.

Windows may still show a blue box: **Windows protected your PC**. It shows
this for programs it hasn't seen many people download yet, signed or not.
Choose **More info**, check it says **TheCodeSaiyan Ltd**, then **Run
anyway**.

### 3. Say yes to Windows

Windows asks whether this program may make changes. Choose **Yes**. You may
need the administrator password here.

KeyJutsu asks for this because it installs for everyone on the computer. One
part of it sometimes runs with extra permission, and it's safest where only an
administrator can change it.

### 4. Answer two questions

- **Add the keyjutsu command to your PATH?** Choose **Yes**. This lets you
  type `keyjutsu` in any terminal.
- **Add "Open KeyJutsu here" to Explorer?** Choose **Yes** if you'd like to
  right-click a folder and open KeyJutsu there. It's fine to say **No**.

The installer may then download a part of Windows it needs, called
WebView2. That needs the internet. Windows 11 usually has it already.

### 5. Open a terminal

A **terminal** is a window where you type commands instead of clicking.

Press the Windows key, type `Terminal`, and press Enter. You should see a
window with a line of text and a blinking cursor.

If a terminal was already open, close it and open a new one. Old windows
don't know KeyJutsu was just installed.

### 6. Check your computer

Type this and press Enter:

```powershell
keyjutsu doctor
```

You should see a list. Each line starts with `[ok  ]`, `[warn]` or `[FAIL]`.

- `[ok  ]` means that part is ready.
- `[warn]` means something useful is missing. On a new computer that's
  normal for **PowerShell 7**, **Git** and **AI agents**. KeyJutsu works
  without them, and you can still try the demo.
- `[FAIL]` means something KeyJutsu needs isn't working. The line says what.

When something is missing, the line below it says where to get it: a web
address on the maker's own site. In the KeyJutsu app, the same line has a
link you can click.

## If it goes wrong

- **"keyjutsu is not recognized".** Close the terminal and open a new one.
  If it still happens, run the installer again and choose **Yes** for the
  PATH question.
- **The installer stops straight away.** It may need the internet to fetch
  WebView2. Connect, and try again.

## Removing it

Open **Settings**, then **Apps**, then **Installed apps**. Find **KeyJutsu**,
choose the three dots next to it, then **Uninstall**.

Next: [your first performance](first-performance.md).
