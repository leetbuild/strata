// StrataProbe: the SKSE plugin behind oracle check 7 (docs/DESIGN.md, "Oracle checks").
//
// When the game has loaded its data (SKSE's kDataLoaded message) it writes
// Data\SKSE\Plugins\strata-probe.json and ends the game. The file says what the game
// really loaded: its version, the SKSE plugin DLLs in the process, and the game plugins
// (esm/esp/esl) in load order. Through Strata's overlay the file lands in overwrite/.
//
// It only acts when the environment variable STRATA_PROBE is set, so leaving the mod
// enabled never ends a real play session:
//   STRATA_PROBE=1       write the file, then end the game
//   STRATA_PROBE=report  write the file, keep playing
//
// No CommonLibSSE: SKSE's plugin interface is a few plain structs (SKSE64 PluginAPI.h,
// checked against CommonLibSSE's SKSE/Impl/Stubs.h), and the two game addresses used are
// for 1.5.97 only and guarded by the runtime version:
//   TESDataHandler singleton   SkyrimSE.exe + 0x1EBE428   (Address Library id 514141,
//                              read from version-1-5-97-0.bin)
//   TESDataHandler +0xD70      compiledFileCollection { BSTArray files; BSTArray smallFiles }
//   TESFile +0x58              fileName[MAX_PATH]         (CommonLibSSE RE/T/TESDataHandler.h,
//                                                          RE/T/TESFile.h)
//
// Skyrim LE (the 32-bit build, `_M_IX86`): SKSE 1.7.3 for 1.9.32, whose source gives the
// interface (src/skse/skse/PluginAPI.h: the same fields, kInterface_Messaging = 5,
// kMessage_DataLoaded = 8) and the game's memory (src/skse/skse/GameData.cpp/.h):
//   DataHandler singleton      *(DataHandler**)0x012E2CB8 (TESV.exe has no ASLR)
//   DataHandler +0x694         ModList { tList<ModInfo> modInfoList; UInt32 loadedModCount;
//                                        ModInfo* loadedMods[0xFF] }  (loadedModCount at +0x69C)
//   ModInfo +0x20              name[0x104]
// guarded by the runtime version 1.9.32.0 (0x09200000, RUNTIME_VERSION_1_9_32_0).
//
// Build: tools/probe/build.sh (clang-cl + xwin); `x86` for LE.

#define WIN32_LEAN_AND_MEAN
#define PSAPI_VERSION 2
#include <windows.h>

#include <psapi.h>

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

namespace
{

// ---- SKSE's plugin interface (the parts used) ----
using PluginHandle = std::uint32_t;

struct PluginInfo
{
  std::uint32_t infoVersion;  // 1
  const char* name;
  std::uint32_t version;
};

struct SKSEInterface
{
  std::uint32_t skseVersion;
  std::uint32_t runtimeVersion;
  std::uint32_t editorVersion;
  std::uint32_t isEditor;
  void* (*QueryInterface)(std::uint32_t id);
  PluginHandle (*GetPluginHandle)();
  std::uint32_t (*GetReleaseIndex)();
  const void* (*GetPluginInfo)(const char* name);
};

struct Message
{
  const char* sender;
  std::uint32_t type;
  std::uint32_t dataLen;
  void* data;
};

struct SKSEMessagingInterface
{
  std::uint32_t interfaceVersion;
  bool (*RegisterListener)(PluginHandle listener, const char* sender, void (*handler)(Message*));
  bool (*Dispatch)(PluginHandle, std::uint32_t, void*, std::uint32_t, const char*);
  void* (*GetEventDispatcher)(std::uint32_t);
};

constexpr std::uint32_t kInterfaceMessaging = 5;
constexpr std::uint32_t kMessageDataLoaded  = 8;
#if !defined(_M_IX86)
constexpr std::uint32_t kRuntime_1_5_97     = 0x01050610;  // (1 << 24) | (5 << 16) | (97 << 4)
#endif

#if defined(_M_IX86)
// ---- the game's memory, Skyrim LE 1.9.32 only (SKSE 1.7.3 source) ----
constexpr std::uint32_t kRuntime_1_9_32       = 0x09200000;
constexpr std::uintptr_t kLeDataHandler       = 0x012E2CB8;
constexpr std::size_t kLeLoadedModCount       = 0x69C;
constexpr std::size_t kLeLoadedMods           = 0x6A0;
constexpr std::size_t kLeModName              = 0x20;
#else
// ---- the game's memory, 1.5.97 only ----
constexpr std::uintptr_t kDataHandlerSingleton = 0x1EBE428;
constexpr std::size_t kCompiledFiles           = 0xD70;
constexpr std::size_t kCompiledSmallFiles      = 0xD88;
constexpr std::size_t kFileName                = 0x58;

struct BSTArray  // BSTArray<TESFile*>
{
  void** data;
  std::uint32_t capacity;
  std::uint32_t pad;
  std::uint32_t size;
};
#endif

std::uint32_t g_runtime = 0;
std::uint32_t g_skse    = 0;

std::string utf8(const wchar_t* text)
{
  const int size = WideCharToMultiByte(CP_UTF8, 0, text, -1, nullptr, 0, nullptr, nullptr);
  std::string out(size > 0 ? size - 1 : 0, '\0');
  if (size > 1) {
    WideCharToMultiByte(CP_UTF8, 0, text, -1, out.data(), size, nullptr, nullptr);
  }
  return out;
}

std::string jsonString(const std::string& text)
{
  std::string out = "\"";
  for (const unsigned char c : text) {
    if (c == '"' || c == '\\') {
      out += '\\';
      out += static_cast<char>(c);
    } else if (c < 0x20) {
      char buffer[8];
      std::snprintf(buffer, sizeof(buffer), "\\u%04x", c);
      out += buffer;
    } else {
      out += static_cast<char>(c);
    }
  }
  return out + "\"";
}

std::string jsonList(const std::vector<std::string>& items)
{
  std::string out = "[";
  for (std::size_t i = 0; i < items.size(); ++i) {
    out += (i ? ", " : "") + jsonString(items[i]);
  }
  return out + "]";
}

// The DLLs loaded from ...\SKSE\Plugins\, by file name.
std::vector<std::string> sksePlugins()
{
  std::vector<std::string> names;
  HMODULE modules[2048];
  DWORD needed = 0;
  if (!EnumProcessModules(GetCurrentProcess(), modules, sizeof(modules), &needed)) {
    return names;
  }
  const std::size_t count = (needed < sizeof(modules) ? needed : sizeof(modules)) / sizeof(HMODULE);
  for (std::size_t i = 0; i < count; ++i) {
    wchar_t path[MAX_PATH * 2];
    if (GetModuleFileNameW(modules[i], path, MAX_PATH * 2) == 0) {
      continue;
    }
    std::wstring lower = path;
    for (wchar_t& c : lower) {
      c = static_cast<wchar_t>(towlower(c));
    }
    if (lower.find(L"\\skse\\plugins\\") != std::wstring::npos) {
      const wchar_t* name = wcsrchr(path, L'\\');
      names.push_back(utf8(name ? name + 1 : path));
    }
  }
  return names;
}

#if !defined(_M_IX86)
void fileNames(const BSTArray& files, std::vector<std::string>& out)
{
  if (files.data == nullptr || files.size > 4096) {
    return;
  }
  for (std::uint32_t i = 0; i < files.size; ++i) {
    const char* file = static_cast<const char*>(files.data[i]);
    if (file != nullptr) {
      out.emplace_back(file + kFileName, strnlen(file + kFileName, MAX_PATH));
    }
  }
}

#endif

double secondsSinceProcessStart()
{
  FILETIME created, exited, kernel, user, now;
  if (!GetProcessTimes(GetCurrentProcess(), &created, &exited, &kernel, &user)) {
    return -1.0;
  }
  GetSystemTimeAsFileTime(&now);
  const auto ticks = [](const FILETIME& t) {
    return (static_cast<std::uint64_t>(t.dwHighDateTime) << 32) | t.dwLowDateTime;
  };
  return static_cast<double>(ticks(now) - ticks(created)) / 1e7;
}

void writeReport()
{
  std::vector<std::string> full, light;
  bool readFiles = false;
#if defined(_M_IX86)
  if (g_runtime == kRuntime_1_9_32) {
    const auto handler = *reinterpret_cast<const char* const*>(kLeDataHandler);
    if (handler != nullptr) {
      const auto count = *reinterpret_cast<const std::uint32_t*>(handler + kLeLoadedModCount);
      const auto mods  = reinterpret_cast<const char* const*>(handler + kLeLoadedMods);
      for (std::uint32_t i = 0; i < count && i < 0xFF; ++i) {
        if (mods[i] != nullptr) {
          full.emplace_back(mods[i] + kLeModName, strnlen(mods[i] + kLeModName, 0x104));
        }
      }
      readFiles = true;
    }
  }
#else
  if (g_runtime == kRuntime_1_5_97) {
    const auto base    = reinterpret_cast<std::uintptr_t>(GetModuleHandleW(nullptr));
    const auto handler = *reinterpret_cast<const char**>(base + kDataHandlerSingleton);
    if (handler != nullptr) {
      fileNames(*reinterpret_cast<const BSTArray*>(handler + kCompiledFiles), full);
      fileNames(*reinterpret_cast<const BSTArray*>(handler + kCompiledSmallFiles), light);
      readFiles = true;
    }
  }
#endif

  wchar_t exe[MAX_PATH * 2];
  GetModuleFileNameW(nullptr, exe, MAX_PATH * 2);
  std::wstring path = exe;
  path.resize(path.find_last_of(L'\\') + 1);
  const std::wstring folder = path + L"Data\\SKSE\\Plugins";
  CreateDirectoryW((path + L"Data\\SKSE").c_str(), nullptr);
  CreateDirectoryW(folder.c_str(), nullptr);

  char runtime[32], skse[32], seconds[32];
#if defined(_M_IX86)
  // SKSE 1.x leaves the leading 1 out (MAKE_SKYRIM_VERSION(9, 32, 0) is 1.9.32.0).
  std::snprintf(runtime, sizeof(runtime), "1.%u.%u.%u", g_runtime >> 24, (g_runtime >> 16) & 0xFF,
                (g_runtime >> 4) & 0xFFF);
#else
  std::snprintf(runtime, sizeof(runtime), "%u.%u.%u.%u", g_runtime >> 24, (g_runtime >> 16) & 0xFF,
                (g_runtime >> 4) & 0xFFF, g_runtime & 0xF);
#endif
  std::snprintf(skse, sizeof(skse), "%u.%u.%u", g_skse >> 24, (g_skse >> 16) & 0xFF, (g_skse >> 4) & 0xFFF);
  std::snprintf(seconds, sizeof(seconds), "%.3f", secondsSinceProcessStart());

  std::string json = "{\n";
  json += "  \"probe\": 1,\n";
  json += "  \"game_version\": " + jsonString(runtime) + ",\n";
  json += "  \"skse_version\": " + jsonString(skse) + ",\n";
  json += "  \"exe\": " + jsonString(utf8(exe)) + ",\n";
  json += "  \"seconds_to_data_loaded\": " + std::string(seconds) + ",\n";
  json += "  \"skse_plugins\": " + jsonList(sksePlugins()) + ",\n";
  json += std::string("  \"plugins_read\": ") + (readFiles ? "true" : "false") + ",\n";
  json += "  \"plugins\": " + jsonList(full) + ",\n";
  json += "  \"light_plugins\": " + jsonList(light) + "\n";
  json += "}\n";

  // Written under a temporary name first: a reader never sees half a file.
  const std::wstring target = folder + L"\\strata-probe.json";
  const std::wstring temp   = target + L".tmp";
  const HANDLE file = CreateFileW(temp.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) {
    return;
  }
  DWORD written = 0;
  WriteFile(file, json.data(), static_cast<DWORD>(json.size()), &written, nullptr);
  CloseHandle(file);
  MoveFileExW(temp.c_str(), target.c_str(), MOVEFILE_REPLACE_EXISTING);
}

void onMessage(Message* message)
{
  if (message == nullptr || message->type != kMessageDataLoaded) {
    return;
  }
  char mode[16] = {};
  if (GetEnvironmentVariableA("STRATA_PROBE", mode, sizeof(mode)) == 0) {
    return;
  }
  writeReport();
  if (std::strcmp(mode, "report") != 0) {
    TerminateProcess(GetCurrentProcess(), 0);
  }
}

}  // namespace

extern "C" __declspec(dllexport) bool SKSEPlugin_Query(const SKSEInterface* skse, PluginInfo* info)
{
  info->infoVersion = 1;
  info->name        = "StrataProbe";
  info->version     = 1;
  return skse != nullptr && skse->isEditor == 0;
}

extern "C" __declspec(dllexport) bool SKSEPlugin_Load(const SKSEInterface* skse)
{
  g_runtime = skse->runtimeVersion;
  g_skse    = skse->skseVersion;
  const auto* messaging = static_cast<const SKSEMessagingInterface*>(skse->QueryInterface(kInterfaceMessaging));
  if (messaging == nullptr) {
    return false;
  }
  return messaging->RegisterListener(skse->GetPluginHandle(), "SKSE", onMessage);
}
