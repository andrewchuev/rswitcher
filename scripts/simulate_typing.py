import ctypes
import time
import random
import sys

# Win32 Constants
LANG_EN = 0x0409
LANG_RU = 0x0419
LANG_UA = 0x0422

# Virtual Key Codes
VK_SHIFT = 0x10
VK_CONTROL = 0x11
VK_MENU = 0x12  # Alt

VK_MAP = {
    'a': 0x41, 'b': 0x42, 'c': 0x43, 'd': 0x44, 'e': 0x45, 'f': 0x46, 'g': 0x47, 'h': 0x48,
    'i': 0x49, 'j': 0x4A, 'k': 0x4B, 'l': 0x4C, 'm': 0x4D, 'n': 0x4E, 'o': 0x4F, 'p': 0x50,
    'q': 0x51, 'r': 0x52, 's': 0x53, 't': 0x54, 'u': 0x55, 'v': 0x56, 'w': 0x57, 'x': 0x58,
    'y': 0x59, 'z': 0x5A,
    ' ': 0x20, '\n': 0x0D, '\t': 0x09, ';': 0xBA, '=': 0xBB, ',': 0xBC, '-': 0xBD, '.': 0xBE,
    '/': 0xBF, '`': 0xC0, '[': 0xDB, '\\': 0xDC, ']': 0xDD, "'": 0xDE,
}

def char_to_vk(char):
    if 'a' <= char <= 'z':
        return VK_MAP[char], False
    elif 'A' <= char <= 'Z':
        return VK_MAP[char.lower()], True
    elif char in VK_MAP:
        return VK_MAP[char], False
    
    shifted = {
        '~': '`', '!': '1', '@': '2', '#': '3', '$': '4', '%': '5', '^': '6', '&': '7', '*': '8', '(': '9', ')': '0',
        '_': '-', '+': '=', '{': '[', '}': ']', '|': '\\', ':': ';', '"': "'", '<': ',', '>': '.', '?': '/'
    }
    if char in shifted:
        base = shifted[char]
        if '0' <= base <= '9':
            vk = 0x30 + int(base)
        else:
            vk = VK_MAP[base]
        return vk, True
    
    if '0' <= char <= '9':
        return 0x30 + int(char), False
        
    return None, False

def type_string(text):
    for char in text:
        vk, shift = char_to_vk(char)
        if vk is None:
            # Skip unsupported characters
            continue
        
        if shift:
            ctypes.windll.user32.keybd_event(VK_SHIFT, 0, 0, 0)
        
        # Key down
        ctypes.windll.user32.keybd_event(vk, 0, 0, 0)
        
        # Simulate slight hold time
        time.sleep(random.uniform(0.015, 0.035))
        
        # Key up
        ctypes.windll.user32.keybd_event(vk, 0, 2, 0) # KEYEVENTF_KEYUP = 2
        
        if shift:
            ctypes.windll.user32.keybd_event(VK_SHIFT, 0, 2, 0)
            
        # Typist delay between keystrokes
        time.sleep(random.uniform(0.04, 0.12))

def get_active_layout():
    hwnd = ctypes.windll.user32.GetForegroundWindow()
    thread_id = ctypes.windll.user32.GetWindowThreadProcessId(hwnd, None)
    hkl = ctypes.windll.user32.GetKeyboardLayout(thread_id)
    return hkl & 0xFFFF

def set_active_layout(lang_id):
    klid_map = {
        LANG_EN: "00000409",
        LANG_RU: "00000419",
        LANG_UA: "00000422"
    }
    klid = klid_map.get(lang_id)
    if not klid:
        return False
    
    # Load and activate layout
    hkl = ctypes.windll.user32.LoadKeyboardLayoutW(klid, 1) # KLF_ACTIVATE = 1
    if not hkl:
        return False
        
    hwnd = ctypes.windll.user32.GetForegroundWindow()
    # Post WM_INPUTLANGCHANGEREQUEST (0x50)
    ctypes.windll.user32.PostMessageW(hwnd, 0x50, 0, hkl)
    time.sleep(0.5)  # wait for layout switch
    return get_active_layout() == lang_id

def lang_name(lang_id):
    if lang_id == LANG_EN: return "EN"
    if lang_id == LANG_RU: return "RU"
    if lang_id == LANG_UA: return "UA"
    return f"Unknown ({hex(lang_id)})"

# Test Cases
# Note: input_str must be represented in keys corresponding to US keyboard layout.
# If typing in EN layout: "ghbdtn " -> RSwitcher should detect it and switch layout.
# If typing in RU layout: "ghbdtn " (which outputs "привет ") -> RSwitcher should NOT switch layout (it's already correct).
TEST_CASES = [
    {
        "name": "Auto-switch EN -> RU ('ghbdtn ')",
        "initial_lang": LANG_EN,
        "input_str": "ghbdtn ",
        "expected_lang": LANG_RU,
        "description": "Typing 'ghbdtn' (привет) in EN layout should switch layout to RU."
    },
    {
        "name": "Auto-switch EN -> UA ('ghjdbn ')",
        "initial_lang": LANG_EN,
        "input_str": "ghjdbn ",
        "expected_lang": LANG_UA,
        "description": "Typing 'ghjdbn' (привіт) in EN layout should switch layout to UA."
    },
    {
        "name": "No switch on correct EN ('hello ')",
        "initial_lang": LANG_EN,
        "input_str": "hello ",
        "expected_lang": LANG_EN,
        "description": "Typing correct EN word 'hello' in EN layout should keep layout EN."
    },
    {
        "name": "No switch on correct RU ('ghbdtn ' typed in RU)",
        "initial_lang": LANG_RU,
        "input_str": "ghbdtn ", # typed as physical keys, produces 'привет'
        "expected_lang": LANG_RU,
        "description": "Typing 'привет' in RU layout should keep layout RU."
    },
    {
        "name": "Auto-switch EN -> RU with capitalized word ('Ghbdtn ')",
        "initial_lang": LANG_EN,
        "input_str": "Ghbdtn ",
        "expected_lang": LANG_RU,
        "description": "Typing capitalized 'Ghbdtn' (Привет) in EN layout should switch layout to RU."
    },
    {
        "name": "No switch on code/commands ('select * from users; ')",
        "initial_lang": LANG_EN,
        "input_str": "select * from users; ",
        "expected_lang": LANG_EN,
        "description": "Typing SQL query in EN layout should keep layout EN (no switch)."
    }
]

def main():
    print("=" * 60)
    print("           RSwitcher E2E Input Simulation Tester")
    print("=" * 60)
    print("This script will simulate user typing to test RSwitcher's algorithms.")
    print("Make sure RSwitcher is running in the background.\n")
    print("IMPORTANT:")
    print("1. Open Notepad (or another text editor).")
    print("2. Focus the text editor (click cursor inside it).")
    print("3. DO NOT type or move your mouse/focus during the test.\n")
    
    for i in range(5, 0, -1):
        print(f"Starting in {i} seconds... (Focus your text editor now!)", end="\r")
        time.sleep(1)
    print("\nStarting tests...\n")
    
    passed = 0
    total = len(TEST_CASES)
    
    for idx, tc in enumerate(TEST_CASES, 1):
        print(f"Test {idx}/{total}: {tc['name']}")
        print(f"  Description: {tc['description']}")
        
        # 1. Setup initial layout
        print(f"  Setting initial layout to {lang_name(tc['initial_lang'])}...")
        if not set_active_layout(tc['initial_lang']):
            print(f"  [ERROR] Failed to set initial layout to {lang_name(tc['initial_lang'])}.")
            print(f"  Please ensure you have EN, RU and UA layouts installed in Windows.")
            print("-" * 50)
            continue
            
        time.sleep(0.3)
        
        # Double check current layout
        cur_lang = get_active_layout()
        if cur_lang != tc['initial_lang']:
            print(f"  [ERROR] Active layout is {lang_name(cur_lang)}, but expected {lang_name(tc['initial_lang'])}.")
            print("-" * 50)
            continue
            
        # 2. Simulate typing
        print(f"  Typing keys: '{tc['input_str']}'")
        type_string(tc['input_str'])
        
        # 3. Wait for RSwitcher to detect and switch
        time.sleep(0.5)
        
        # 4. Check layout after typing
        final_lang = get_active_layout()
        success = (final_lang == tc['expected_lang'])
        
        if success:
            print(f"  [PASS] Layout is {lang_name(final_lang)} (Expected: {lang_name(tc['expected_lang'])})")
            passed += 1
        else:
            print(f"  [FAIL] Layout is {lang_name(final_lang)} (Expected: {lang_name(tc['expected_lang'])})")
            
        print("-" * 50)
        time.sleep(1.0) # Gap between tests

    print("=" * 60)
    print(f"Test Results: {passed}/{total} passed")
    print("=" * 60)

if __name__ == "__main__":
    main()
