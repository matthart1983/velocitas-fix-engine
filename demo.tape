Output demo.gif

Set Shell "bash"
Set FontSize 13
Set Width 1200
Set Height 800
Set Padding 15
Set Theme "Catppuccin Mocha"
Set TypingSpeed 50ms
Set PlaybackSpeed 1

Type "cargo run --release --bin aeron_demo"
Enter
Sleep 12s

Sleep 2s
Type "clear"
Enter
Sleep 1s

Type "cargo run --release --bin aeron_demo benchmark"
Enter
Sleep 8s
