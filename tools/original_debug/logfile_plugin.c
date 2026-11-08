/* x32dbg plugin, for the private copy in ~/oblivion-capture only: every log line goes to C:\x32log.txt with a
   millisecond tick, instead of the Log tab. This 2020 build has no GuiLogRedirect and no command for it, and input
   doesn't reach the GUI under the virtual KWin, so GuiAddLogMessage jumps here.
   Build: clang --target=i686-w64-mingw32 -O1 -fno-builtin -nostdlib -shared -fuse-ld=lld -Wl,--entry,_DllMainCRTStartup@12
          -o logfile.dp32 logfile_plugin.c /usr/lib/wine/i386-windows/libkernel32.a */
typedef void *HANDLE;
#define IMPORT __declspec(dllimport)
IMPORT HANDLE __stdcall GetModuleHandleA(const char *);
IMPORT void *__stdcall GetProcAddress(HANDLE, const char *);
IMPORT int __stdcall VirtualProtect(void *, unsigned long, unsigned long, unsigned long *);
IMPORT HANDLE __stdcall CreateFileA(const char *, unsigned long, unsigned long, void *, unsigned long, unsigned long, HANDLE);
IMPORT int __stdcall WriteFile(HANDLE, const void *, unsigned long, unsigned long *, void *);
IMPORT int __stdcall FlushInstructionCache(HANDLE, const void *, unsigned long);
IMPORT HANDLE __stdcall GetCurrentProcess(void);

static HANDLE file;
static unsigned long(__stdcall *tick_count)(void);

static void __cdecl to_file(const char *text)
{
    char tick[16];
    unsigned long value = tick_count(), wrote, n = 0;
    int i = 10;
    tick[10] = ' ';
    while (i > 0) {
        tick[--i] = (char)('0' + value % 10);
        value /= 10;
    }
    WriteFile(file, tick, 11, &wrote, 0);
    while (text[n]) n++;
    WriteFile(file, text, n, &wrote, 0);
}

__declspec(dllexport) int __cdecl pluginit(int *init)
{
    unsigned char *target = (unsigned char *)GetProcAddress(GetModuleHandleA("x32bridge.dll"), "GuiAddLogMessage");
    unsigned long old;
    tick_count = (unsigned long(__stdcall *)(void))GetProcAddress(GetModuleHandleA("kernel32.dll"), "GetTickCount");
    file = CreateFileA("C:\\x32log.txt", 0x40000000, 1, 0, 2, 0x80, 0);
    if (target && file != (HANDLE)-1 && VirtualProtect(target, 5, 0x40, &old)) {
        target[0] = 0xE9;
        *(long *)(target + 1) = (long)((unsigned char *)to_file - (target + 5));
        FlushInstructionCache(GetCurrentProcess(), target, 5);
    }
    init[0] = 1;
    init[1] = 8;
    return 1;
}
__declspec(dllexport) int __cdecl plugstop(void) { return 1; }
__declspec(dllexport) void __cdecl plugsetup(void *setup) { (void)setup; }
int __stdcall DllMainCRTStartup(void *a, unsigned b, void *c) { (void)a; (void)b; (void)c; return 1; }
