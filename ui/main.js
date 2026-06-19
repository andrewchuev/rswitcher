// ── Tauri IPC invocation helper ───────────────────────────────────────────
const { invoke } = window.__TAURI__.core;

// ── DOM Elements ───────────────────────────────────────────────────────────
const masterSwitch = document.getElementById('master-switch');
const exclusionsList = document.getElementById('exclusions-list');
const manualInput = document.getElementById('manual-input');
const addBtn = document.getElementById('add-btn');
const runningAppsSelect = document.getElementById('running-apps-select');
const hotkeyToggle = document.getElementById('hotkey-toggle');
const hotkeyDisplay = document.getElementById('hotkey-display');
const undoHotkeyToggle = document.getElementById('undo-hotkey-toggle');
const undoHotkeyDisplay = document.getElementById('undo-hotkey-display');
const autostartToggle = document.getElementById('autostart-toggle');
const langSelect = document.getElementById('lang-select');
const sensitivitySelect = document.getElementById('sensitivity-select');
const openConfigBtn = document.getElementById('open-config-btn');

// New settings elements
const selectionReplaceToggle = document.getElementById('selection-replace-toggle');
const devExclusionsList = document.getElementById('dev-exclusions-list');
const devManualInput = document.getElementById('dev-manual-input');
const devAddBtn = document.getElementById('dev-add-btn');
const devRunningAppsSelect = document.getElementById('dev-running-apps-select');
const ignoredWordsList = document.getElementById('ignored-words-list');
const clearIgnoredBtn = document.getElementById('clear-ignored-btn');

// Local cached state
let settings = null;
let runningApps = [];
let currentLang = 'en';
let recordingHotkey = null; // null or { target, displayEl, originalText, originalVk, originalWin }
let winActive = false;

// ── Translations ───────────────────────────────────────────────────────────
const translations = {
  en: {
    title: "RSwitcher — Settings",
    masterLabel: "Auto-switching",
    exclusionsTitle: "Exclusions (processes)",
    exclusionsSubtitle: "do not switch layout in these windows",
    exclusionsEmpty: "exclusions list is empty",
    manualLabel: "Manual:",
    manualPlaceholder: "app.exe",
    addButton: "Add",
    runningLabel: "Running:",
    runningSelectPlaceholder: "Select running...",
    hotkeysTitle: "Hotkeys",
    hotkeyForceLabel: "Force switch",
    hotkeyUndoLabel: "Undo last switch",
    hotkeysFooter: "Click the key combination to change it.",
    pressKeys: "Press keys...",
    systemTitle: "System",
    autostartLabel: "Start on Windows startup",
    langLabel: "Language:",
    footerNote: "Settings are saved automatically.",
    openConfigBtn: "Open Config",
    noRunningApps: "No running windows",
    alreadyAddedSuffix: " (added)",
    sensitivityLabel: "Sensitivity:",
    sensitivityLow: "Low",
    sensitivityMedium: "Medium",
    sensitivityHigh: "High",
    selectionReplaceLabel: "Use Ctrl+Shift+Left for word deletion",
    devExclusionsTitle: "Development Apps",
    devExclusionsSubtitle: "low sensitivity + min length 5 in these windows",
    devExclusionsEmpty: "development apps list is empty",
    ignoredWordsTitle: "Ignored Words",
    ignoredWordsSubtitle: "words whitelisted by the Undo hotkey",
    clearIgnoredBtn: "Clear Whitelist",
    ignoredEmpty: "whitelist is empty",
    tabGeneral: "General",
    tabHotkeys: "Hotkeys",
    tabExclusions: "Exceptions"
  },
  ru: {
    title: "RSwitcher — Настройки",
    masterLabel: "Автопереключение",
    exclusionsTitle: "Исключения (процессы)",
    exclusionsSubtitle: "не переключать раскладку в этих окнах",
    exclusionsEmpty: "список исключений пуст",
    manualLabel: "Вручную:",
    manualPlaceholder: "app.exe",
    addButton: "Добавить",
    runningLabel: "Запущенные:",
    runningSelectPlaceholder: "Выбрать запущенное...",
    hotkeysTitle: "Горячие клавиши",
    hotkeyForceLabel: "Принудительное переключение",
    hotkeyUndoLabel: "Отменить последнее переключение",
    hotkeysFooter: "Нажмите на комбинацию клавиш, чтобы изменить её.",
    pressKeys: "Нажмите клавиши...",
    systemTitle: "Система",
    autostartLabel: "Запускать при старте Windows",
    langLabel: "Язык:",
    footerNote: "Настройки сохраняются автоматически.",
    openConfigBtn: "Открыть конфиг",
    noRunningApps: "Нет запущенных окон",
    alreadyAddedSuffix: " (добавлено)",
    sensitivityLabel: "Чувствительность:",
    sensitivityLow: "Низкая",
    sensitivityMedium: "Средняя",
    sensitivityHigh: "Высокая",
    selectionReplaceLabel: "Выделять слово через Ctrl+Shift+Left при замене",
    devExclusionsTitle: "Среды разработки и терминалы",
    devExclusionsSubtitle: "пониженная чувствительность и мин. длина 5 в этих окнах",
    devExclusionsEmpty: "список сред разработки пуст",
    ignoredWordsTitle: "Исключенные слова",
    ignoredWordsSubtitle: "слова, добавленные в белый список через Отмену",
    clearIgnoredBtn: "Очистить список",
    ignoredEmpty: "список пуст",
    tabGeneral: "Основные",
    tabHotkeys: "Горячие клавиши",
    tabExclusions: "Исключения"
  },
  uk: {
    title: "RSwitcher — Налаштування",
    masterLabel: "Автоперемикання",
    exclusionsTitle: "Винятки (процеси)",
    exclusionsSubtitle: "не перемикати розкладку в цих вікнах",
    exclusionsEmpty: "список винятків порожній",
    manualLabel: "Вручну:",
    manualPlaceholder: "app.exe",
    addButton: "Додати",
    runningLabel: "Запущені:",
    runningSelectPlaceholder: "Обрати запущену...",
    hotkeysTitle: "Гарячі клавіші",
    hotkeyForceLabel: "Примусове перемикання",
    hotkeyUndoLabel: "Скасувати останнє перемикання",
    hotkeysFooter: "Натисніть на комбінацію клавіш, щоб змінити її.",
    pressKeys: "Натисніть клавіші...",
    systemTitle: "Система",
    autostartLabel: "Запускать разом з Windows",
    langLabel: "Мова:",
    footerNote: "Налаштування зберігаються автоматично.",
    openConfigBtn: "Відкрити конфіг",
    noRunningApps: "Немає запущених вікон",
    alreadyAddedSuffix: " (додано)",
    sensitivityLabel: "Чутливість:",
    sensitivityLow: "Низька",
    sensitivityMedium: "Середня",
    sensitivityHigh: "Висока",
    selectionReplaceLabel: "Виділяти слово через Ctrl+Shift+Left при заміні",
    devExclusionsTitle: "Середовища розробки та термінали",
    devExclusionsSubtitle: "знижена чутливість та мін. довжина 5 в цих вікнах",
    devExclusionsEmpty: "список середовищ розробки порожній",
    ignoredWordsTitle: "Виключені слова",
    ignoredWordsSubtitle: "слова, додані до білого списку через Скасування",
    clearIgnoredBtn: "Очистити список",
    ignoredEmpty: "список порожній",
    tabGeneral: "Основні",
    tabHotkeys: "Гарячі клавіші",
    tabExclusions: "Винятки"
  }
};

// ── Virtual Key mapping to display name ────────────────────────────────────
function vkDisplayName(vk, win) {
  let base = "";
  if (vk >= 65 && vk <= 90) {
    base = String.fromCharCode(vk);
  } else if (vk >= 48 && vk <= 57) {
    base = String.fromCharCode(vk);
  } else if (vk >= 96 && vk <= 105) {
    base = `Num ` + (vk - 96);
  } else if (vk >= 112 && vk <= 135) {
    base = `F` + (vk - 111);
  } else {
    switch (vk) {
      case 0x10: case 0xA0: case 0xA1: base = "Shift"; break;
      case 0x11: case 0xA2: case 0xA3: base = "Ctrl"; break;
      case 0x12: case 0xA4: case 0xA5: base = "Alt"; break;
      case 0x08: base = "Backspace"; break;
      case 0x09: base = "Tab"; break;
      case 0x0D: base = "Enter"; break;
      case 0x13: base = "Pause"; break;
      case 0x14: base = "Caps Lock"; break;
      case 0x1B: base = "Esc"; break;
      case 0x20: base = "Space"; break;
      case 0x21: base = "Page Up"; break;
      case 0x22: base = "Page Down"; break;
      case 0x23: base = "End"; break;
      case 0x24: base = "Home"; break;
      case 0x25: base = "Left"; break;
      case 0x26: base = "Up"; break;
      case 0x27: base = "Right"; break;
      case 0x28: base = "Down"; break;
      case 0x2C: base = "Print Screen"; break;
      case 0x2D: base = "Insert"; break;
      case 0x2E: base = "Delete"; break;
      case 0x5D: base = "Context Menu"; break;
      case 0x90: base = "Num Lock"; break;
      case 0x91: base = "Scroll Lock"; break;
      case 186: base = ";"; break;
      case 187: base = "="; break;
      case 188: base = ","; break;
      case 189: base = "-"; break;
      case 190: base = "."; break;
      case 191: base = "/"; break;
      case 192: base = "`"; break;
      case 219: base = "["; break;
      case 220: base = "\\"; break;
      case 221: base = "]"; break;
      case 222: base = "'"; break;
      default:
        base = `0x${vk.toString(16).toUpperCase()}`;
    }
  }
  return win ? `Win+${base}` : base;
}

// ── Apply translations to the UI ───────────────────────────────────────────
function applyTranslations() {
  const dict = translations[currentLang] || translations['en'];

  // Update document title
  document.title = dict.title;

  // Translate all elements with data-i18n attribute
  document.querySelectorAll('[data-i18n]').forEach(el => {
    const key = el.getAttribute('data-i18n');
    if (dict[key]) {
      el.textContent = dict[key];
    }
  });

  // Translate elements with data-i18n-placeholder
  document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
    const key = el.getAttribute('data-i18n-placeholder');
    if (dict[key]) {
      el.setAttribute('placeholder', dict[key]);
    }
  });

  // Re-render lists and dropdowns with dynamic translated text
  renderUI();
  renderRunningApps();
}

// ── Render UI based on loaded settings ──────────────────────────────────────
function renderUI() {
  if (!settings) return;

  const dict = translations[currentLang] || translations['en'];

  // Master switch
  masterSwitch.checked = settings.enabled;

  // Render exclusions list
  exclusionsList.innerHTML = '';
  if (settings.exceptions.length === 0) {
    const emptyState = document.createElement('div');
    emptyState.className = 'list-empty-state';
    emptyState.textContent = dict.exclusionsEmpty;
    exclusionsList.appendChild(emptyState);
  } else {
    settings.exceptions.forEach((exc, index) => {
      const li = document.createElement('li');
      
      const bullet = document.createElement('span');
      bullet.className = 'bullet';
      bullet.textContent = '•';
      
      const nameSpan = document.createElement('span');
      nameSpan.className = 'app-name';
      nameSpan.textContent = exc;
      
      const delBtn = document.createElement('button');
      delBtn.className = 'delete-btn';
      delBtn.textContent = '✕';
      delBtn.addEventListener('click', () => deleteException(index));
      
      li.appendChild(bullet);
      li.appendChild(nameSpan);
      li.appendChild(delBtn);
      exclusionsList.appendChild(li);
    });
  }

  // Hotkey controls
  hotkeyToggle.checked = settings.hotkey_enabled;
  if (!recordingHotkey || recordingHotkey.target !== 'hotkey') {
    hotkeyDisplay.textContent = vkDisplayName(settings.hotkey_vk, settings.hotkey_win);
  }
  
  if (settings.hotkey_enabled) {
    hotkeyToggle.closest('.checkbox-row').classList.remove('disabled');
  } else {
    hotkeyToggle.closest('.checkbox-row').classList.add('disabled');
    if (recordingHotkey && recordingHotkey.target === 'hotkey') {
      stopRecording(false);
    }
  }

  // Undo hotkey controls
  undoHotkeyToggle.checked = settings.undo_hotkey_enabled;
  if (!recordingHotkey || recordingHotkey.target !== 'undo_hotkey') {
    undoHotkeyDisplay.textContent = vkDisplayName(settings.undo_hotkey_vk, settings.undo_hotkey_win);
  }
  
  if (settings.undo_hotkey_enabled) {
    undoHotkeyToggle.closest('.checkbox-row').classList.remove('disabled');
  } else {
    undoHotkeyToggle.closest('.checkbox-row').classList.add('disabled');
    if (recordingHotkey && recordingHotkey.target === 'undo_hotkey') {
      stopRecording(false);
    }
  }

  // Selection replacement checkbox
  selectionReplaceToggle.checked = settings.use_selection_replace;

  // Render dev exclusions list
  devExclusionsList.innerHTML = '';
  if (settings.dev_exceptions.length === 0) {
    const emptyState = document.createElement('div');
    emptyState.className = 'list-empty-state';
    emptyState.textContent = dict.devExclusionsEmpty;
    devExclusionsList.appendChild(emptyState);
  } else {
    settings.dev_exceptions.forEach((exc, index) => {
      const li = document.createElement('li');
      
      const bullet = document.createElement('span');
      bullet.className = 'bullet';
      bullet.textContent = '•';
      
      const nameSpan = document.createElement('span');
      nameSpan.className = 'app-name';
      nameSpan.textContent = exc;
      
      const delBtn = document.createElement('button');
      delBtn.className = 'delete-btn';
      delBtn.textContent = '✕';
      delBtn.addEventListener('click', () => deleteDevException(index));
      
      li.appendChild(bullet);
      li.appendChild(nameSpan);
      li.appendChild(delBtn);
      devExclusionsList.appendChild(li);
    });
  }

  // Render ignored words whitelist as pill tags
  ignoredWordsList.innerHTML = '';
  if (settings.ignored_words.length === 0) {
    const emptyState = document.createElement('div');
    emptyState.className = 'list-empty-state';
    emptyState.textContent = dict.ignoredEmpty;
    ignoredWordsList.appendChild(emptyState);
  } else {
    settings.ignored_words.forEach((word, index) => {
      const tag = document.createElement('span');
      tag.className = 'ignored-tag';
      tag.textContent = word + ' ';
      
      const removeBtn = document.createElement('button');
      removeBtn.className = 'remove-tag-btn';
      removeBtn.textContent = '✕';
      removeBtn.addEventListener('click', () => removeIgnoredWord(index));
      
      tag.appendChild(removeBtn);
      ignoredWordsList.appendChild(tag);
    });
  }
}

// ── Render Running Apps select dropdown ─────────────────────────────────────
function renderRunningApps() {
  const dict = translations[currentLang] || translations['en'];
  
  // Keep the first option
  runningAppsSelect.innerHTML = `<option value="" disabled selected>${dict.runningSelectPlaceholder}</option>`;
  devRunningAppsSelect.innerHTML = `<option value="" disabled selected>${dict.runningSelectPlaceholder}</option>`;
  
  if (runningApps.length === 0) {
    const opt1 = document.createElement('option');
    opt1.disabled = true;
    opt1.textContent = dict.noRunningApps;
    runningAppsSelect.appendChild(opt1);

    const opt2 = document.createElement('option');
    opt2.disabled = true;
    opt2.textContent = dict.noRunningApps;
    devRunningAppsSelect.appendChild(opt2);
  } else {
    runningApps.forEach(app => {
      const alreadyAdded = settings && settings.exceptions.includes(app.exe.toLowerCase());
      const opt = document.createElement('option');
      opt.value = app.exe;
      opt.textContent = alreadyAdded ? `${app.exe}${dict.alreadyAddedSuffix}` : app.exe;
      opt.disabled = alreadyAdded;
      runningAppsSelect.appendChild(opt);

      const alreadyAddedDev = settings && settings.dev_exceptions.includes(app.exe.toLowerCase());
      const optDev = document.createElement('option');
      optDev.value = app.exe;
      optDev.textContent = alreadyAddedDev ? `${app.exe}${dict.alreadyAddedSuffix}` : app.exe;
      optDev.disabled = alreadyAddedDev;
      devRunningAppsSelect.appendChild(optDev);
    });
  }
}

// ── Hotkey Recording Logic ──────────────────────────────────────────────────
function startRecording(target, displayEl) {
  if (recordingHotkey) {
    stopRecording(false); // cancel any active recording first
  }
  
  const originalText = displayEl.textContent;
  let originalVk, originalWin;
  if (target === 'hotkey') {
    originalVk = settings.hotkey_vk;
    originalWin = settings.hotkey_win;
  } else {
    originalVk = settings.undo_hotkey_vk;
    originalWin = settings.undo_hotkey_win;
  }
  
  recordingHotkey = {
    target,
    displayEl,
    originalText,
    originalVk,
    originalWin
  };
  
  winActive = false;
  displayEl.classList.add('recording');
  const dict = translations[currentLang] || translations['en'];
  displayEl.textContent = dict.pressKeys;
  
  // Attach capture phase event listeners to override default hotkeys and shortcuts
  window.addEventListener('keydown', onKeyDownRecording, true);
  window.addEventListener('keyup', onKeyUpRecording, true);
  window.addEventListener('blur', onBlurRecording);
  document.addEventListener('click', onClickOutsideRecording, true);
}

function stopRecording(save, vk = 0, win = false) {
  if (!recordingHotkey) return;
  
  const { target, displayEl, originalText } = recordingHotkey;
  
  // Remove event listeners
  window.removeEventListener('keydown', onKeyDownRecording, true);
  window.removeEventListener('keyup', onKeyUpRecording, true);
  window.removeEventListener('blur', onBlurRecording);
  document.removeEventListener('click', onClickOutsideRecording, true);
  
  displayEl.classList.remove('recording');
  recordingHotkey = null;
  winActive = false;
  
  if (save) {
    if (target === 'hotkey') {
      settings.hotkey_vk = vk;
      settings.hotkey_win = win;
    } else {
      settings.undo_hotkey_vk = vk;
      settings.undo_hotkey_win = win;
    }
    
    // Save settings back to Tauri
    invoke('save_settings', { settings })
      .then((saved) => {
        if (saved) settings = saved;
        renderUI();
      })
      .catch(err => {
        console.error("Failed to save hotkeys:", err);
        renderUI();
      });
  } else {
    displayEl.textContent = originalText;
  }
}

function onKeyDownRecording(e) {
  e.preventDefault();
  e.stopPropagation();
  
  const keyCode = e.keyCode;
  
  // Abort recording if Escape (27) is pressed
  if (keyCode === 27) {
    stopRecording(false);
    return;
  }
  
  // If the key is the Meta/Windows key (91 or 92), display "Win + ..." and wait for the rest
  if (keyCode === 91 || keyCode === 92) {
    winActive = true;
    if (recordingHotkey) {
      recordingHotkey.displayEl.textContent = 'Win + ...';
    }
    return;
  }
  
  // Otherwise, finish recording and capture keycode + meta state
  const win = winActive || e.metaKey;
  stopRecording(true, keyCode, win);
}

function onKeyUpRecording(e) {
  const keyCode = e.keyCode;
  if (keyCode === 91 || keyCode === 92) {
    winActive = false;
    if (recordingHotkey) {
      const dict = translations[currentLang] || translations['en'];
      recordingHotkey.displayEl.textContent = dict.pressKeys;
    }
  }
}

function onBlurRecording() {
  stopRecording(false);
}

function onClickOutsideRecording(e) {
  if (recordingHotkey && !recordingHotkey.displayEl.contains(e.target)) {
    stopRecording(false);
  }
}

// ── API Actions ─────────────────────────────────────────────────────────────
async function loadAllData() {
  try {
    settings = await invoke('get_settings');
    const autostart = await invoke('is_autostart_enabled');
    autostartToggle.checked = autostart;
    
    if (settings && settings.lang) {
      currentLang = settings.lang;
      langSelect.value = currentLang;
    } else {
      currentLang = 'en';
      langSelect.value = 'en';
    }
    
    if (settings && settings.sensitivity !== undefined) {
      sensitivitySelect.value = settings.sensitivity.toFixed(1);
    } else {
      sensitivitySelect.value = '1.0';
    }
    
    applyTranslations();
    await refreshRunningApps();
  } catch (err) {
    console.error("Failed to load settings:", err);
  }
}

async function refreshRunningApps() {
  try {
    runningApps = await invoke('get_running_apps');
    renderRunningApps();
  } catch (err) {
    console.error("Failed to fetch running apps:", err);
  }
}

async function addManualException() {
  const value = manualInput.value.trim().toLowerCase();
  if (!value) return;
  
  try {
    settings = await invoke('add_exception', { app: value });
    manualInput.value = '';
    renderUI();
    renderRunningApps();
  } catch (err) {
    console.error("Failed to add exception:", err);
  }
}

async function deleteException(index) {
  try {
    settings = await invoke('remove_exception', { index });
    renderUI();
    renderRunningApps();
  } catch (err) {
    console.error("Failed to delete exception:", err);
  }
}

async function addManualDevException() {
  const value = devManualInput.value.trim().toLowerCase();
  if (!value) return;
  if (!settings) return;
  if (!settings.dev_exceptions.includes(value)) {
    settings.dev_exceptions.push(value);
    try {
      const saved = await invoke('save_settings', { settings });
      if (saved) settings = saved;
      devManualInput.value = '';
      renderUI();
      renderRunningApps();
    } catch (err) {
      console.error("Failed to add dev exception:", err);
    }
  }
}

async function deleteDevException(index) {
  if (!settings) return;
  settings.dev_exceptions.splice(index, 1);
  try {
    const saved = await invoke('save_settings', { settings });
    if (saved) settings = saved;
    renderUI();
    renderRunningApps();
  } catch (err) {
    console.error("Failed to delete dev exception:", err);
  }
}

async function removeIgnoredWord(index) {
  if (!settings) return;
  settings.ignored_words.splice(index, 1);
  try {
    const saved = await invoke('save_settings', { settings });
    if (saved) settings = saved;
    renderUI();
  } catch (err) {
    console.error("Failed to remove ignored word:", err);
  }
}

async function clearIgnoredWords() {
  if (!settings) return;
  settings.ignored_words = [];
  try {
    const saved = await invoke('save_settings', { settings });
    if (saved) settings = saved;
    renderUI();
  } catch (err) {
    console.error("Failed to clear ignored words:", err);
  }
}

// ── Event Listeners ─────────────────────────────────────────────────────────

// Master Switch
masterSwitch.addEventListener('change', async () => {
  try {
    settings = await invoke('set_enabled', { enabled: masterSwitch.checked });
    renderUI();
  } catch (err) {
    console.error("Failed to toggle master switch:", err);
  }
});

// Manual Input Add
addBtn.addEventListener('click', addManualException);
manualInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    addManualException();
  }
});

// Running Apps ComboBox
runningAppsSelect.addEventListener('change', async () => {
  const selectedApp = runningAppsSelect.value;
  if (!selectedApp) return;
  
  try {
    settings = await invoke('add_exception', { app: selectedApp });
    runningAppsSelect.value = ''; // Reset dropdown selection
    renderUI();
    renderRunningApps();
  } catch (err) {
    console.error("Failed to add selected app exception:", err);
  }
});

// Hotkey Toggles
hotkeyToggle.addEventListener('change', async () => {
  if (!settings) return;
  settings.hotkey_enabled = hotkeyToggle.checked;
  try {
    const saved = await invoke('save_settings', { settings });
    if (saved) settings = saved;
    renderUI();
  } catch (err) {
    console.error("Failed to save hotkey settings:", err);
  }
});

undoHotkeyToggle.addEventListener('change', async () => {
  if (!settings) return;
  settings.undo_hotkey_enabled = undoHotkeyToggle.checked;
  try {
    const saved = await invoke('save_settings', { settings });
    if (saved) settings = saved;
    renderUI();
  } catch (err) {
    console.error("Failed to save undo hotkey settings:", err);
  }
});

// Autostart Toggle
autostartToggle.addEventListener('change', async () => {
  try {
    await invoke('set_autostart', { enabled: autostartToggle.checked });
  } catch (err) {
    console.error("Failed to save autostart settings:", err);
  }
});

// Language Select Dropdown
langSelect.addEventListener('change', async () => {
  if (!settings) return;
  settings.lang = langSelect.value;
  currentLang = settings.lang;
  applyTranslations();
  try {
    const saved = await invoke('save_settings', { settings });
    if (saved) settings = saved;
  } catch (err) {
    console.error("Failed to save settings with new language:", err);
  }
});

// Sensitivity Select Dropdown
sensitivitySelect.addEventListener('change', async () => {
  if (!settings) return;
  settings.sensitivity = parseFloat(sensitivitySelect.value);
  try {
    const saved = await invoke('save_settings', { settings });
    if (saved) settings = saved;
  } catch (err) {
    console.error("Failed to save sensitivity settings:", err);
  }
});

// Selection replace toggle listener
selectionReplaceToggle.addEventListener('change', async () => {
  if (!settings) return;
  settings.use_selection_replace = selectionReplaceToggle.checked;
  try {
    const saved = await invoke('save_settings', { settings });
    if (saved) settings = saved;
    renderUI();
  } catch (err) {
    console.error("Failed to save selection replace settings:", err);
  }
});

// Development Exceptions listeners
devAddBtn.addEventListener('click', addManualDevException);
devManualInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    addManualDevException();
  }
});

devRunningAppsSelect.addEventListener('change', async () => {
  const selectedApp = devRunningAppsSelect.value;
  if (!selectedApp) return;
  if (!settings) return;
  const value = selectedApp.toLowerCase();
  if (!settings.dev_exceptions.includes(value)) {
    settings.dev_exceptions.push(value);
    try {
      const saved = await invoke('save_settings', { settings });
      if (saved) settings = saved;
      devRunningAppsSelect.value = '';
      renderUI();
      renderRunningApps();
    } catch (err) {
      console.error("Failed to add selected app to dev exceptions:", err);
    }
  }
});

// Whitelist clear listener
clearIgnoredBtn.addEventListener('click', clearIgnoredWords);

// Keycap Clicks (Interactive Recording)
hotkeyDisplay.addEventListener('click', () => {
  if (settings && settings.hotkey_enabled) {
    startRecording('hotkey', hotkeyDisplay);
  }
});

undoHotkeyDisplay.addEventListener('click', () => {
  if (settings && settings.undo_hotkey_enabled) {
    startRecording('undo_hotkey', undoHotkeyDisplay);
  }
});

let pollIntervalId = null;

function startPolling() {
  if (!pollIntervalId) {
    refreshRunningApps();
    pollIntervalId = setInterval(refreshRunningApps, 3000);
  }
}

function stopPolling() {
  if (pollIntervalId) {
    clearInterval(pollIntervalId);
    pollIntervalId = null;
  }
}

// Open Config button
openConfigBtn.addEventListener('click', async () => {
  try {
    await invoke('open_config_dir');
  } catch (err) {
    console.error("Failed to open config directory:", err);
  }
});

// ── Initialization ─────────────────────────────────────────────────────────
document.addEventListener('DOMContentLoaded', () => {
  loadAllData();
  
  // Tabs Navigation switching logic
  document.querySelectorAll('.tab-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      const tabName = btn.getAttribute('data-tab');
      document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
      document.querySelectorAll('.tab-content').forEach(c => c.classList.remove('active'));
      document.getElementById(`tab-${tabName}`).classList.add('active');
    });
  });

  // Poll running apps list only when the window has focus to save CPU/battery
  if (document.hasFocus()) {
    startPolling();
  }
  window.addEventListener('focus', startPolling);
  window.addEventListener('blur', stopPolling);
  
  // Refresh immediately when the dropdown gains focus
  runningAppsSelect.addEventListener('focus', refreshRunningApps);
  devRunningAppsSelect.addEventListener('focus', refreshRunningApps);
});
