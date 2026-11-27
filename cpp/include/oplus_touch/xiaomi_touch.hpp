#pragma once

#include <array>
#include <cstdint>
#include <mutex>
#include <optional>
#include <string>

namespace oplus::touch {

enum class XiaomiTouchCmd : uint32_t {
    SetCurValue = 0,
    GetCurValue = 1,
    GetDefValue = 2,
    GetMinValue = 3,
    GetMaxValue = 4,
    GetModeValue = 5,
    ResetMode = 6,
    SetLongValue = 7,
};

enum class XiaomiTouchMode : uint32_t {
    GameMode = 0,
    ActiveMode = 1,
    UpThreshold = 2,
    Tolerance = 3,
    WghMin = 4,
    WghMax = 5,
    WghStep = 6,
    EdgeFilter = 7,
    PanelOrientation = 8,
    ReportRate = 9,
    FodEnable = 10,
    AodEnable = 11,
    ResistRf = 12,
    IdleTime = 13,
    DoubletapMode = 14,
    GripMode = 15,
    ModeNum = 16,
};

inline constexpr size_t kXiaomiValueTypes = 6;
using XiaomiModeTable = std::array<std::array<int32_t, kXiaomiValueTypes>,
                                   static_cast<size_t>(XiaomiTouchMode::ModeNum)>;

class XiaomiTouch {
  public:
    static XiaomiTouch open_optional(const std::string& path);
    explicit XiaomiTouch(std::string path);
    ~XiaomiTouch();

    XiaomiTouch(const XiaomiTouch&) = delete;
    XiaomiTouch& operator=(const XiaomiTouch&) = delete;
    XiaomiTouch(XiaomiTouch&& other) noexcept;
    XiaomiTouch& operator=(XiaomiTouch&& other) noexcept;

    bool is_available() const { return fd_ >= 0; }
    int descriptor() const { return fd_; }
    const std::string& path() const { return path_; }

    std::optional<int32_t> ioctl(XiaomiTouchCmd command, XiaomiTouchMode mode, int32_t value);
    std::optional<int32_t> get_mode(XiaomiTouchMode mode);
    std::optional<int32_t> set_mode(XiaomiTouchMode mode, int32_t value);

    bool enable_aod(bool enable);
    bool enable_dt2w(bool enable);
    bool enable_fod(bool enable);
    bool set_report_rate(int32_t rate);
    bool set_game_mode(bool enable);

    XiaomiModeTable all_modes() const;
    static unsigned long ioctl_request(XiaomiTouchCmd command);

  private:
    static XiaomiModeTable initial_mode_table();
    void initialize();
    void close_device();

    int fd_ = -1;
    std::string path_;
    mutable std::mutex mutex_;
    XiaomiModeTable mode_cache_{};
};

}  // namespace oplus::touch
