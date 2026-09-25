# Your first performance

**By the end of this page:** you've mashed your keyboard and watched real
commands type themselves out perfectly, and you know how to stop.

This is safe. The demo only **reads** information about your computer. It
changes nothing.

## Before you start

KeyJutsu is installed. [Installing KeyJutsu](installing.md) shows how.

## Steps

### 1. Open a terminal

Press the Windows key, type `Terminal`, and press Enter.

### 2. Start the demo

Type this and press Enter:

```powershell
keyjutsu demo --clean
```

`--clean` means "start plainly". It hides your own terminal settings for now,
so nothing unexpected shows up.

You should see a line ending in `>`, with a blinking cursor after it. That's
called the **prompt**. It means the terminal is waiting.

### 3. Mash the keyboard

Press any letters, fast or slow, it doesn't matter which.

Each key you press types **one letter of a real command**. Not the letter you
pressed: the next letter of the command KeyJutsu has ready. So however you
mash, the command comes out right.

When a command is finished, the next key you press runs it. You'll see
information about your computer appear. While it's running, your keys are
ignored, so nothing you press can get in the way.

There are three commands. Together they take a few hundred key presses.

### 4. Take the terminal back

When the third command has finished, hold down these four keys together:

**Ctrl** + **Alt** + **Shift** + **K**

This is called **disarming**. It hands the terminal back to you. You can do
it at any moment, not only at the end. It always works.

### 5. Leave

Type `exit` and press Enter.

KeyJutsu shows a short report. Each command should say **succeeded**.

## What just happened

- The commands were real, and so was what they showed you.
- The typing was theatre. Your keys only decided **when** each letter
  appeared, never **which** letter.
- KeyJutsu knew each command had finished because the terminal told it, not
  because it guessed.

## Keys worth remembering

- **Ctrl + Alt + Shift + K**: stop, and take the terminal back.
- **Ctrl + C**: interrupt a command that's running. This always works too.

Next: [running a plan safely](running-a-plan.md).
