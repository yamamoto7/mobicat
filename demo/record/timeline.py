"""The demo, second by second.

Times are seconds from the start of the recording. Every action that shows up
on screen (a tap on a phone, a key in the terminal) comes from this list, so
the phones and the terminal stay in step on every take.

Actions:
  ("type", text)          type into the terminal, one character at a time
  ("key", name, label)    press a tmux key (Enter, Tab, C-c, …); label is shown on screen
  ("android", link)       open a shelf:// link on the emulator (taps a button)
  ("android-launch",)     start the app on the emulator
  ("ios", name)           post com.example.shelf.<name> on the simulator (taps a button)
  ("caption", text, secs) show a caption
"""

APP = "com.example.shelf"

TIMELINE = [
    # 1. Title (drawn by compose.py over the first seconds)
    (4.0, "caption", "Android and iOS app logs, in one terminal.", 3.8),
    # 2. Pick the app
    (4.2, "type", "mobicat"),
    (5.3, "key", "Enter", "⏎"),
    (8.0, "caption", "Run mobicat and pick your app.", 4.0),
    (8.4, "type", "sh"),
    (10.0, "key", "Enter", "⏎"),
    # 3. Logs from both phones, one timeline
    (11.0, "caption", "Both phones, one timeline. Only your app's logs.", 10.5),
    (12.0, "android", "add?item=0"),
    (13.2, "ios", "add1"),
    (14.6, "android", "add?item=3"),
    (16.0, "ios", "sync"),
    (17.6, "android", "sync"),
    (19.2, "ios", "add4"),
    # 4. Filter
    (22.0, "caption", "Filter as you type, with completion.", 10.0),
    (22.2, "key", "/", "/"),
    (22.8, "type", "lev"),
    (23.6, "key", "Tab", "Tab"),
    (24.3, "type", "w"),
    (24.8, "key", "Tab", "Tab"),
    (25.6, "key", "Enter", "⏎"),
    (26.6, "ios", "sync"),
    (28.0, "android", "sync"),
    (32.0, "key", "/", "/"),
    (32.4, "key", "C-u", "^U"),
    (32.9, "key", "Enter", "⏎"),
    # 5. Crash on Android
    (34.0, "caption", "Crashes stand out, with the full stack trace.", 11.0),
    (34.2, "android", "add?item=1"),
    (36.0, "android", "checkout"),
    # 6. Restart
    (45.5, "caption", "Restart the app. mobicat keeps following it.", 6.5),
    (46.0, "android-launch",),
    (48.5, "android", "add?item=2"),
    # 7. A crash on the other phone
    (52.0, "caption", "Watching one phone? A crash on the other still shows up.", 12.0),
    (52.4, "key", "Tab", "Tab"),
    (55.0, "ios", "add3"),
    (56.8, "ios", "checkout"),
    (60.5, "key", "Tab", "Tab"),
    # 8. Detail and copy
    (64.0, "caption", "Open the details. Copy it into a bug report, or to your AI agent.", 10.0),
    (64.4, "key", "[", "["),
    (64.9, "key", "c", "c"),
    (66.4, "key", "Enter", "⏎"),
    (69.6, "key", "y", "y"),
    (71.8, "key", "Escape", "Esc"),
    (72.6, "key", "Escape", "Esc"),
    # 9. Script it
    (74.0, "key", "C-c", "^C"),
    (74.5, "key", "C-c", "^C"),
    (74.8, "caption", "Script it: JSON for CI and agents.", 10.0),
    (75.4, "type", f"mobicat {APP} --json -q is:crash | jq -r .message"),
    (79.0, "key", "Enter", "⏎"),
    (85.0, "key", "C-c", "^C"),
]

# 10. Outro card
END = 90.0
