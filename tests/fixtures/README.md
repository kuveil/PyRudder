# 测试夹具 / Test fixtures

Fake Python 运行时、命令辅助程序、wheel 和畸形产物放在此处。

Fake Python runtimes, command helpers, wheels, and malformed artifacts belong here.

`fake_runtime.rs` 为 Windows 命令发现集成测试创建独立临时运行时；`pe_header.rs` 提供合成的 PE32+ 文件头，复用于扫描和有界解析测试。合成 PE 不是可执行程序；扫描测试不运行脚本。实际进程转发需要后续独立的原生 Console/GUI helper。

`fake_runtime.rs` creates isolated temporary runtimes for Windows command-discovery integration tests. `pe_header.rs` supplies synthetic PE32+ headers shared by scanning and bounded-parser tests. Synthetic PE files are not executable programs, and discovery tests never run scripts. Actual forwarding requires separate native Console/GUI helpers in a later milestone.
