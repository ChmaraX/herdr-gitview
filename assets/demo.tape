# Demo recording for the README gif.
# Requires: vhs (https://github.com/charmbracelet/vhs), a repo with a few
# staged/unstaged/untracked changes, and the plugin built.
#
#   vhs assets/demo.tape
#
# Records the standalone list + a scripted browse; for the full two-pane
# demo (editor loop, notes), record inside herdr with your screen recorder
# of choice and export to assets/demo.gif.

Output assets/demo.gif

Set FontSize 15
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set Padding 12

Hide
Type "cd $(mktemp -d) && git init -q demo && cd demo"
Enter
Type "printf 'fn main() {\n    println!(\"hello\");\n}\n' > main.rs && git add . && git commit -qm init"
Enter
Type "printf 'fn main() {\n    println!(\"hello, world\");\n    run();\n}\n' > main.rs"
Enter
Type "printf 'notes\n' > scratch.md && printf 'fn run() {}\n' > lib.rs && git add lib.rs"
Enter
Type "clear"
Enter
Show

Type "herdr-gitview list"
Enter
Sleep 2s

# browse
Type "j"
Sleep 800ms
Type "j"
Sleep 800ms
Type "k"
Sleep 800ms

# stage the modified file, watch it move sections
Type "s"
Sleep 1.5s
# unstage it again
Type "u"
Sleep 1.5s

# help overlay
Type "?"
Sleep 2s
Escape
Sleep 500ms

# history
Type "l"
Sleep 1.5s
Escape
Sleep 800ms

Type "q"
Sleep 500ms
