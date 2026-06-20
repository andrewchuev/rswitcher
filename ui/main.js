'use strict';
const { invoke } = window.__TAURI__.core;

// ── State ─────────────────────────────────────────────────────────────────────
let settings = null;
let runningApps = [];
let currentLang = 'en';
let recordingTarget = null; // null | 'hotkey' | 'undo_hotkey'
let winActive = false;
let recOrigText = '';
let pollId = null;

// ── DOM ───────────────────────────────────────────────────────────────────────
const masterSwitch    = document.getElementById('master-switch');
const autostartToggle = document.getElementById('autostart-toggle');
const langSelect      = document.getElementById('lang-select');
const openConfigBtn   = document.getElementById('open-config-btn');
const hotkeyEnabled   = document.getElementById('hotkey-enabled');
const hotkeyKbd       = document.getElementById('hotkey-kbd');
const undoEnabled     = document.getElementById('undo-enabled');
const undoKbd         = document.getElementById('undo-kbd');
const wordInput       = document.getElementById('word-input');
const wordAddBtn      = document.getElementById('word-add-btn');
const wordClearBtn    = document.getElementById('word-clear-btn');
const ignoredChips    = document.getElementById('ignored-chips');
const appInput        = document.getElementById('app-input');
const appAddBtn       = document.getElementById('app-add-btn');
const appsList        = document.getElementById('apps-list');
const runningSelect   = document.getElementById('running-apps-select');
const wordsBadge      = document.getElementById('words-badge');
const appsBadge       = document.getElementById('apps-badge');
const adminStatus     = document.getElementById('admin-status');
const adminOk         = document.getElementById('admin-ok');
const restartAdminBtn = document.getElementById('restart-admin-btn');
const themeBtn        = document.getElementById('theme-btn');
const themeIcon       = document.getElementById('theme-icon');
const themeLabel      = document.getElementById('theme-label');

// ── Translations ──────────────────────────────────────────────────────────────
const T = {
  en: {
    title: 'RSwitcher — Settings',
    tabDetection: 'Detection', tabHotkeys: 'Hotkeys',
    tabWords: 'Ignored words', tabApps: 'App exclusions', tabSystem: 'System',
    detectionTitle: 'Language detection',
    sensitivityLabel: 'Sensitivity', sensitivityDesc: 'How aggressively words are detected',
    sensitivityLow: 'Low', sensitivityMedium: 'Medium', sensitivityHigh: 'High',
    cyrillicLabel: 'Cyrillic preference', cyrillicDesc: 'How RU vs UA ambiguity is resolved',
    cyrillicAuto: 'Auto', cyrillicRu: 'Russian', cyrillicUa: 'Ukrainian',
    deletionLabel: 'Word deletion', deletionDesc: 'How the mistyped word is erased',
    deletionBackspace: 'Backspace', deletionSelect: 'Ctrl+Shift+←',
    hotkeysTitle: 'Hotkeys',
    hotkeyForceLabel: 'Force switch', hotkeyForceDesc: 'Re-type word in the other layout',
    hotkeyUndoLabel: 'Undo last switch', hotkeyUndoDesc: 'Restore original word and layout',
    hotkeysHint: 'Click a key combination to change it. Only the Win modifier is supported.',
    wordsTitle: 'Ignored words', wordsPlaceholder: 'type a word…', wordsEmpty: 'whitelist is empty',
    addButton: 'Add', clearAllButton: 'Clear all',
    appsTitle: 'App exclusions', appsPlaceholder: 'app.exe', appsEmpty: 'no exclusions',
    runningLabel: 'Add from running apps', runningPlaceholder: 'Select running app…',
    alreadyAddedSuffix: ' (added)', noRunningApps: 'No running windows',
    systemTitle: 'System', autostartLabel: 'Start with Windows', langLabel: 'Interface language',
    configLabel: 'Config file', configPath: '%APPDATA%\\rswitcher\\config.json', openConfigBtn: 'Open',
    adminWarningText: "Standard user — won't work in admin windows",
    adminOkText: 'Running as administrator', restartAdminBtn: 'Restart as admin',
    savedNote: 'Saved automatically', themeLight: 'Light', themeDark: 'Dark',
    pressKeys: 'Press keys…',
  },
  ru: {
    title: 'RSwitcher — Настройки',
    tabDetection: 'Детекция', tabHotkeys: 'Хоткеи',
    tabWords: 'Игнор. слова', tabApps: 'Исключения', tabSystem: 'Система',
    detectionTitle: 'Определение языка',
    sensitivityLabel: 'Чувствительность', sensitivityDesc: 'Агрессивность автопереключения',
    sensitivityLow: 'Низкая', sensitivityMedium: 'Средняя', sensitivityHigh: 'Высокая',
    cyrillicLabel: 'Кириллица', cyrillicDesc: 'Предпочтение при неоднозначности RU/UA',
    cyrillicAuto: 'Авто', cyrillicRu: 'Русский', cyrillicUa: 'Украинский',
    deletionLabel: 'Удаление слова', deletionDesc: 'Как стирается ошибочное слово',
    deletionBackspace: 'Backspace', deletionSelect: 'Ctrl+Shift+←',
    hotkeysTitle: 'Горячие клавиши',
    hotkeyForceLabel: 'Принудительное переключение', hotkeyForceDesc: 'Перепечатать слово в другой раскладке',
    hotkeyUndoLabel: 'Отменить переключение', hotkeyUndoDesc: 'Восстановить слово и раскладку',
    hotkeysHint: 'Нажмите на комбинацию клавиш, чтобы изменить её. Поддерживается только модификатор Win.',
    wordsTitle: 'Исключённые слова', wordsPlaceholder: 'введите слово…', wordsEmpty: 'список пуст',
    addButton: 'Добавить', clearAllButton: 'Очистить',
    appsTitle: 'Исключения приложений', appsPlaceholder: 'app.exe', appsEmpty: 'нет исключений',
    runningLabel: 'Добавить из запущенных', runningPlaceholder: 'Выбрать запущенное…',
    alreadyAddedSuffix: ' (добавлено)', noRunningApps: 'Нет запущенных окон',
    systemTitle: 'Система', autostartLabel: 'Запускать вместе с Windows', langLabel: 'Язык интерфейса',
    configLabel: 'Файл конфигурации', configPath: '%APPDATA%\\rswitcher\\config.json', openConfigBtn: 'Открыть',
    adminWarningText: 'Обычный пользователь — не работает в окнах администратора',
    adminOkText: 'Запущено как администратор', restartAdminBtn: 'Запуск от имени Админа',
    savedNote: 'Настройки сохраняются автоматически',
    themeLight: 'Светлая', themeDark: 'Тёмная', pressKeys: 'Нажмите клавиши…',
  },
  uk: {
    title: 'RSwitcher — Налаштування',
    tabDetection: 'Детекція', tabHotkeys: 'Гарячі клав.',
    tabWords: 'Ігнор. слова', tabApps: 'Винятки', tabSystem: 'Система',
    detectionTitle: 'Визначення мови',
    sensitivityLabel: 'Чутливість', sensitivityDesc: 'Агресивність автоперемикання',
    sensitivityLow: 'Низька', sensitivityMedium: 'Середня', sensitivityHigh: 'Висока',
    cyrillicLabel: 'Кирилиця', cyrillicDesc: 'Перевага при неоднозначності RU/UA',
    cyrillicAuto: 'Авто', cyrillicRu: 'Російська', cyrillicUa: 'Українська',
    deletionLabel: 'Видалення слова', deletionDesc: 'Як стирається помилково введене слово',
    deletionBackspace: 'Backspace', deletionSelect: 'Ctrl+Shift+←',
    hotkeysTitle: 'Гарячі клавіші',
    hotkeyForceLabel: 'Примусове перемикання', hotkeyForceDesc: 'Передрукувати слово в іншій розкладці',
    hotkeyUndoLabel: 'Скасувати перемикання', hotkeyUndoDesc: 'Відновити слово і розкладку',
    hotkeysHint: 'Натисніть комбінацію клавіш, щоб змінити її. Підтримується лише модифікатор Win.',
    wordsTitle: 'Виключені слова', wordsPlaceholder: 'введіть слово…', wordsEmpty: 'список порожній',
    addButton: 'Додати', clearAllButton: 'Очистити',
    appsTitle: 'Винятки додатків', appsPlaceholder: 'app.exe', appsEmpty: 'немає винятків',
    runningLabel: 'Додати з запущених', runningPlaceholder: 'Обрати запущений…',
    alreadyAddedSuffix: ' (додано)', noRunningApps: 'Немає запущених вікон',
    systemTitle: 'Система', autostartLabel: 'Запускати разом з Windows', langLabel: 'Мова інтерфейсу',
    configLabel: 'Файл конфігурації', configPath: '%APPDATA%\\rswitcher\\config.json', openConfigBtn: 'Відкрити',
    adminWarningText: 'Звичайний користувач — не працює у вікнах адміністратора',
    adminOkText: 'Запущено з правами адміністратора', restartAdminBtn: 'Запустити як Адмін',
    savedNote: 'Налаштування зберігаються автоматично',
    themeLight: 'Світла', themeDark: 'Темна', pressKeys: 'Натисніть клавіші…',
  },
};

// ── VK display name ───────────────────────────────────────────────────────────
function vkDisplayName(vk, win) {
  let base = '';
  if (vk >= 65 && vk <= 90) base = String.fromCharCode(vk);
  else if (vk >= 48 && vk <= 57) base = String.fromCharCode(vk);
  else if (vk >= 96 && vk <= 105) base = 'Num ' + (vk - 96);
  else if (vk >= 112 && vk <= 135) base = 'F' + (vk - 111);
  else switch (vk) {
    case 0x10: case 0xA0: case 0xA1: base = 'Shift'; break;
    case 0x11: case 0xA2: case 0xA3: base = 'Ctrl'; break;
    case 0x12: case 0xA4: case 0xA5: base = 'Alt'; break;
    case 0x08: base = 'Backspace'; break;
    case 0x09: base = 'Tab'; break;
    case 0x0D: base = 'Enter'; break;
    case 0x13: base = 'Pause'; break;
    case 0x14: base = 'Caps Lock'; break;
    case 0x1B: base = 'Esc'; break;
    case 0x20: base = 'Space'; break;
    case 0x21: base = 'Page Up'; break;
    case 0x22: base = 'Page Down'; break;
    case 0x23: base = 'End'; break;
    case 0x24: base = 'Home'; break;
    case 0x25: base = '←'; break;
    case 0x26: base = '↑'; break;
    case 0x27: base = '→'; break;
    case 0x28: base = '↓'; break;
    case 0x2C: base = 'Print Screen'; break;
    case 0x2D: base = 'Insert'; break;
    case 0x2E: base = 'Delete'; break;
    case 0x5D: base = 'Menu'; break;
    case 0x90: base = 'Num Lock'; break;
    case 0x91: base = 'Scroll Lock'; break;
    case 186: base = ';'; break;
    case 187: base = '='; break;
    case 188: base = ','; break;
    case 189: base = '-'; break;
    case 190: base = '.'; break;
    case 191: base = '/'; break;
    case 192: base = '`'; break;
    case 219: base = '['; break;
    case 220: base = '\\'; break;
    case 221: base = ']'; break;
    case 222: base = "'"; break;
    default: base = '0x' + vk.toString(16).toUpperCase();
  }
  return win ? 'Win+' + base : base;
}

// ── Theme ─────────────────────────────────────────────────────────────────────
function initTheme() {
  const stored = localStorage.getItem('rsw-theme');
  const dark = stored ? stored === 'dark' : !window.matchMedia('(prefers-color-scheme: light)').matches;
  applyTheme(dark);
}

function applyTheme(dark) {
  document.documentElement.classList.toggle('light', !dark);
  themeIcon.className = dark ? 'ti ti-moon' : 'ti ti-sun';
  const d = T[currentLang] || T.en;
  themeLabel.textContent = dark ? d.themeLight : d.themeDark;
  localStorage.setItem('rsw-theme', dark ? 'dark' : 'light');
}

themeBtn.addEventListener('click', () => {
  applyTheme(document.documentElement.classList.contains('light'));
});

// ── Navigation ────────────────────────────────────────────────────────────────
function showTab(id) {
  document.querySelectorAll('.nav-item').forEach(el =>
    el.classList.toggle('active', el.dataset.tab === id));
  document.querySelectorAll('.tab-panel').forEach(el =>
    el.classList.toggle('active', el.id === 'panel-' + id));
}

document.querySelectorAll('.nav-item').forEach(el =>
  el.addEventListener('click', () => showTab(el.dataset.tab)));

// ── Segmented controls ────────────────────────────────────────────────────────
function syncSeg(group, value) {
  document.querySelectorAll(`[data-group="${group}"] .seg-btn`).forEach(btn =>
    btn.classList.toggle('active', btn.dataset.value === String(value)));
}

document.querySelectorAll('.seg-control').forEach(ctrl => {
  ctrl.querySelectorAll('.seg-btn').forEach(btn =>
    btn.addEventListener('click', () => onSegClick(ctrl.dataset.group, btn.dataset.value)));
});

async function onSegClick(group, value) {
  if (!settings) return;
  syncSeg(group, value);
  if (group === 'sensitivity')  settings.sensitivity = parseFloat(value);
  if (group === 'cyrillic')     settings.preferred_cyrillic = value;
  if (group === 'deletion')     settings.use_selection_replace = (value === 'sel');
  try {
    const s = await invoke('save_settings', { settings });
    if (s) settings = s;
  } catch (e) { console.error(e); }
}

// ── Translations ──────────────────────────────────────────────────────────────
function applyTranslations() {
  const d = T[currentLang] || T.en;
  document.title = d.title;
  document.querySelectorAll('[data-i18n]').forEach(el => {
    const v = d[el.dataset.i18n];
    if (v !== undefined) el.textContent = v;
  });
  document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
    const v = d[el.dataset.i18nPlaceholder];
    if (v !== undefined) el.placeholder = v;
  });
  const dark = !document.documentElement.classList.contains('light');
  themeLabel.textContent = dark ? d.themeLight : d.themeDark;
}

// ── Render ────────────────────────────────────────────────────────────────────
function renderUI() {
  if (!settings) return;
  const d = T[currentLang] || T.en;

  masterSwitch.checked = settings.enabled;

  // Segmented controls
  syncSeg('sensitivity', settings.sensitivity.toFixed(1));
  syncSeg('cyrillic', settings.preferred_cyrillic || 'auto');
  syncSeg('deletion', settings.use_selection_replace ? 'sel' : 'bs');

  // Hotkeys
  hotkeyEnabled.checked = settings.hotkey_enabled;
  undoEnabled.checked   = settings.undo_hotkey_enabled;
  if (recordingTarget !== 'hotkey')     hotkeyKbd.textContent = vkDisplayName(settings.hotkey_vk, settings.hotkey_win);
  if (recordingTarget !== 'undo_hotkey') undoKbd.textContent  = vkDisplayName(settings.undo_hotkey_vk, settings.undo_hotkey_win);
  hotkeyKbd.classList.toggle('disabled', !settings.hotkey_enabled);
  undoKbd.classList.toggle('disabled',   !settings.undo_hotkey_enabled);
  if (!settings.hotkey_enabled     && recordingTarget === 'hotkey')     stopRecording(false);
  if (!settings.undo_hotkey_enabled && recordingTarget === 'undo_hotkey') stopRecording(false);

  // Ignored words chips
  ignoredChips.innerHTML = '';
  if (settings.ignored_words.length === 0) {
    const em = document.createElement('span');
    em.className = 'chips-empty';
    em.textContent = d.wordsEmpty;
    ignoredChips.appendChild(em);
  } else {
    settings.ignored_words.forEach((word, i) => {
      const chip = document.createElement('span');
      chip.className = 'chip';
      const rm = document.createElement('button');
      rm.className = 'chip-rm';
      rm.innerHTML = '<i class="ti ti-x"></i>';
      rm.addEventListener('click', () => removeIgnoredWord(i));
      chip.append(document.createTextNode(word), rm);
      ignoredChips.appendChild(chip);
    });
  }
  wordsBadge.textContent = settings.ignored_words.length > 0 ? settings.ignored_words.length : '';

  // App exclusions list
  appsList.innerHTML = '';
  if (settings.exceptions.length === 0) {
    const em = document.createElement('div');
    em.className = 'apps-empty';
    em.textContent = d.appsEmpty;
    appsList.appendChild(em);
  } else {
    settings.exceptions.forEach((exc, i) => {
      const row = document.createElement('div');
      row.className = 'app-row';
      const rm = document.createElement('button');
      rm.className = 'app-rm';
      rm.innerHTML = '<i class="ti ti-x"></i>';
      rm.addEventListener('click', () => deleteException(i));
      row.append(document.createTextNode(exc), rm);
      appsList.appendChild(row);
    });
  }
  appsBadge.textContent = settings.exceptions.length > 0 ? settings.exceptions.length : '';
}

function renderRunningApps() {
  const d = T[currentLang] || T.en;
  runningSelect.innerHTML = `<option value="" disabled selected>${d.runningPlaceholder}</option>`;
  if (runningApps.length === 0) {
    const opt = document.createElement('option');
    opt.disabled = true;
    opt.textContent = d.noRunningApps;
    runningSelect.appendChild(opt);
  } else {
    runningApps.forEach(app => {
      const added = settings && settings.exceptions.includes(app.exe.toLowerCase());
      const opt = document.createElement('option');
      opt.value = app.exe;
      opt.textContent = added ? app.exe + d.alreadyAddedSuffix : app.exe;
      opt.disabled = added;
      runningSelect.appendChild(opt);
    });
  }
}

// ── Hotkey recording ──────────────────────────────────────────────────────────
function startRecording(target, el) {
  if (recordingTarget) stopRecording(false);
  if (!settings) return;
  if (target === 'hotkey'     && !settings.hotkey_enabled)     return;
  if (target === 'undo_hotkey' && !settings.undo_hotkey_enabled) return;
  recordingTarget = target;
  recOrigText = el.textContent;
  winActive = false;
  el.classList.add('recording');
  el.textContent = (T[currentLang] || T.en).pressKeys;
  window.addEventListener('keydown', onKeyDown, true);
  window.addEventListener('keyup',   onKeyUp,   true);
  window.addEventListener('blur',    onBlur);
  document.addEventListener('click', onClickOut, true);
}

function stopRecording(save, vk = 0, win = false) {
  if (!recordingTarget) return;
  const el = recordingTarget === 'hotkey' ? hotkeyKbd : undoKbd;
  window.removeEventListener('keydown', onKeyDown, true);
  window.removeEventListener('keyup',   onKeyUp,   true);
  window.removeEventListener('blur',    onBlur);
  document.removeEventListener('click', onClickOut, true);
  el.classList.remove('recording');
  if (save) {
    if (recordingTarget === 'hotkey') { settings.hotkey_vk = vk;     settings.hotkey_win = win; }
    else                              { settings.undo_hotkey_vk = vk; settings.undo_hotkey_win = win; }
    invoke('save_settings', { settings })
      .then(s => { if (s) settings = s; renderUI(); })
      .catch(console.error);
  } else {
    el.textContent = recOrigText;
  }
  recordingTarget = null;
  winActive = false;
}

function onKeyDown(e) {
  e.preventDefault(); e.stopPropagation();
  if (e.keyCode === 27) { stopRecording(false); return; }
  if (e.keyCode === 91 || e.keyCode === 92) {
    winActive = true;
    const el = recordingTarget === 'hotkey' ? hotkeyKbd : undoKbd;
    el.textContent = 'Win + …';
    return;
  }
  stopRecording(true, e.keyCode, winActive || e.metaKey);
}

function onKeyUp(e) {
  if (e.keyCode === 91 || e.keyCode === 92) {
    winActive = false;
    const el = recordingTarget === 'hotkey' ? hotkeyKbd : undoKbd;
    if (el) el.textContent = (T[currentLang] || T.en).pressKeys;
  }
}

function onBlur()       { stopRecording(false); }
function onClickOut(e)  {
  const el = recordingTarget === 'hotkey' ? hotkeyKbd : undoKbd;
  if (el && !el.contains(e.target)) stopRecording(false);
}

// ── Data loading ──────────────────────────────────────────────────────────────
async function loadAllData() {
  try {
    settings = await invoke('get_settings');
    const [autostart, elevated, apps] = await Promise.all([
      invoke('is_autostart_enabled'),
      invoke('is_elevated'),
      invoke('get_running_apps'),
    ]);
    autostartToggle.checked = autostart;
    runningApps = apps;

    if (settings && settings.lang) {
      currentLang = settings.lang;
      langSelect.value = currentLang;
    }

    adminStatus.style.display = elevated ? 'none' : 'flex';
    adminOk.style.display     = elevated ? 'flex' : 'none';

    applyTranslations();
    renderUI();
    renderRunningApps();
  } catch (e) { console.error('loadAllData', e); }
}

async function refreshRunningApps() {
  try { runningApps = await invoke('get_running_apps'); renderRunningApps(); }
  catch (e) { console.error(e); }
}

// ── Exception management ──────────────────────────────────────────────────────
async function addException() {
  const val = appInput.value.trim().toLowerCase();
  if (!val) return;
  try {
    settings = await invoke('add_exception', { app: val });
    appInput.value = '';
    renderUI(); renderRunningApps();
  } catch (e) { console.error(e); }
}

async function deleteException(i) {
  try { settings = await invoke('remove_exception', { index: i }); renderUI(); renderRunningApps(); }
  catch (e) { console.error(e); }
}

// ── Ignored words management ──────────────────────────────────────────────────
async function addIgnoredWord() {
  if (!settings) return;
  const word = wordInput.value.trim().toLowerCase();
  if (!word || settings.ignored_words.includes(word)) { wordInput.value = ''; return; }
  settings.ignored_words.push(word);
  try {
    const s = await invoke('save_settings', { settings });
    if (s) settings = s;
    wordInput.value = '';
    renderUI();
  } catch (e) { console.error(e); }
}

async function removeIgnoredWord(i) {
  if (!settings) return;
  settings.ignored_words.splice(i, 1);
  try { const s = await invoke('save_settings', { settings }); if (s) settings = s; renderUI(); }
  catch (e) { console.error(e); }
}

async function clearIgnoredWords() {
  if (!settings) return;
  settings.ignored_words = [];
  try { const s = await invoke('save_settings', { settings }); if (s) settings = s; renderUI(); }
  catch (e) { console.error(e); }
}

// ── Event listeners ───────────────────────────────────────────────────────────
masterSwitch.addEventListener('change', async () => {
  try { settings = await invoke('set_enabled', { enabled: masterSwitch.checked }); renderUI(); }
  catch (e) { console.error(e); }
});

autostartToggle.addEventListener('change', async () => {
  try { await invoke('set_autostart', { enabled: autostartToggle.checked }); }
  catch (e) { console.error(e); }
});

langSelect.addEventListener('change', async () => {
  if (!settings) return;
  settings.lang = langSelect.value;
  currentLang = settings.lang;
  applyTranslations();
  try { const s = await invoke('save_settings', { settings }); if (s) settings = s; }
  catch (e) { console.error(e); }
});

openConfigBtn.addEventListener('click', async () => {
  try { await invoke('open_config_dir'); } catch (e) { console.error(e); }
});

hotkeyEnabled.addEventListener('change', async () => {
  if (!settings) return;
  settings.hotkey_enabled = hotkeyEnabled.checked;
  try { const s = await invoke('save_settings', { settings }); if (s) settings = s; renderUI(); }
  catch (e) { console.error(e); }
});

undoEnabled.addEventListener('change', async () => {
  if (!settings) return;
  settings.undo_hotkey_enabled = undoEnabled.checked;
  try { const s = await invoke('save_settings', { settings }); if (s) settings = s; renderUI(); }
  catch (e) { console.error(e); }
});

hotkeyKbd.addEventListener('click', () => startRecording('hotkey', hotkeyKbd));
undoKbd.addEventListener('click',   () => startRecording('undo_hotkey', undoKbd));

appAddBtn.addEventListener('click', addException);
appInput.addEventListener('keydown', e => { if (e.key === 'Enter') addException(); });

runningSelect.addEventListener('change', async () => {
  const val = runningSelect.value;
  if (!val) return;
  try {
    settings = await invoke('add_exception', { app: val });
    runningSelect.value = '';
    renderUI(); renderRunningApps();
  } catch (e) { console.error(e); }
});

wordAddBtn.addEventListener('click', addIgnoredWord);
wordInput.addEventListener('keydown', e => { if (e.key === 'Enter') addIgnoredWord(); });
wordClearBtn.addEventListener('click', clearIgnoredWords);

restartAdminBtn.addEventListener('click', async () => {
  try { await invoke('restart_as_admin'); } catch (e) { console.error(e); }
});

// ── Running apps polling ──────────────────────────────────────────────────────
function startPolling() {
  if (!pollId) {
    refreshRunningApps();
    pollId = setInterval(refreshRunningApps, 3000);
  }
}

function stopPolling() {
  if (pollId) { clearInterval(pollId); pollId = null; }
}

runningSelect.addEventListener('focus', refreshRunningApps);
window.addEventListener('focus', startPolling);
window.addEventListener('blur',  stopPolling);

// ── Boot ──────────────────────────────────────────────────────────────────────
initTheme();
loadAllData();
if (document.hasFocus()) startPolling();
