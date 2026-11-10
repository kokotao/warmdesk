# 更新日志

## [0.1.0] - 2026-11-10

### 新增

- 新建 WarmDesk macOS 与 Windows 桌面工程。
- 提供强度、工作模式、运行计时和自动停止的交互预览。
- 增加清晰的模拟状态与硬件安全边界说明。
- 设置面板增加栗子镜像优先的更新检查，镜像不可用时回退 GitHub Release。
- 建立 macOS/Windows CI 和 tag 驱动的 GitHub Release 构建流程。

### 当前限制

- 该版本是交互预览，不运行 CPU 工作负载，也不会产生实际热量。
- 温度、电源状态与系统负载监控尚未接入。
- 应用暂未配置 Apple Developer ID 或 Windows Authenticode 签名；系统可能显示未识别开发者/发布者提示。
