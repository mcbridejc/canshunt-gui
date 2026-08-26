import { invoke } from '@tauri-apps/api/core';
import './style.css';
import './controls.css';

const app = document.querySelector('#app');
const state = {
  interfaces: [], connected: false, connection: null, devices: [], selected: null,
  readings: Array.from({ length: 8 }, () => ({ current: null, voltage: null })),
};

const icons = {
  usb: '<svg viewBox="0 0 24 24"><path d="M12 3v13m0-13-2.5 2.5M12 3l2.5 2.5M8 10H5v4a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-4h-3M9 20h6"/></svg>',
  bus: '<svg viewBox="0 0 24 24"><path d="M5 7h14v10H5zM2 10h3m14 0h3M2 14h3m14 0h3M9 7V4m6 3V4M9 20v-3m6 3v-3"/></svg>',
  scan: '<svg viewBox="0 0 24 24"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m15.5 15.5 5 5"/></svg>',
};

app.innerHTML = `
  <header><div class="brand"><div class="logo">CS</div><div><h1>CANShunt</h1><p>Configuration & monitoring</p></div></div><div id="connection-pill" class="pill offline"><i></i>Disconnected</div></header>
  <main>
    <aside>
      <section class="connect-card">
        <div class="section-title"><span>Connection</span><button id="refresh" class="icon-button" title="Refresh interfaces">↻</button></div>
        <label for="interface">CAN interface</label>
        <select id="interface"><option>Discovering interfaces…</option></select>
        <button id="connect" class="primary">Connect</button>
      </section>
      <section class="device-section">
        <div class="section-title"><span>Devices <b id="device-count">0</b></span><button id="scan" class="small" disabled>${icons.scan} Scan bus</button></div>
        <div id="devices" class="device-list"><div class="empty">Connect to an interface,<br>then scan the bus.</div></div>
      </section>
    </aside>
    <div class="content">
      <div id="welcome" class="welcome"><div class="welcome-mark">${icons.bus}</div><h2>No device selected</h2><p>Connect to a CAN interface and scan the bus to find CANShunt devices.</p></div>
      <div id="detail" hidden>
        <div class="device-heading"><div><span class="eyebrow">CANSHUNT DEVICE</span><h2 id="device-name">CANShunt</h2><div id="device-meta"></div></div><div id="nmt-status" class="status-live"><i></i>Unknown</div></div>
        <div class="tabs"><button class="active" data-tab="monitor">Monitor</button><button data-tab="configuration">Configuration</button></div>
        <section id="monitor" class="tab-panel">
          <div class="panel-heading"><div><h3>Live measurements</h3><p>Current and voltage received from configured TPDOs</p></div><span class="receiving"><i></i> Listening</span></div>
          <div class="reading-table"><div class="table-row table-head"><span>Channel</span><span>Current</span><span>Voltage</span></div><div id="readings"></div></div>
        </section>
        <section id="configuration" class="tab-panel" hidden>
          <div class="config-grid">
            <article><div class="panel-heading"><div><h3>Device settings</h3><p>CANopen network configuration</p></div></div>
              <form id="device-form"><label>NMT state<span id="nmt-state-value" class="state-value">Unknown</span></label><button id="nmt-action" class="secondary" type="button">Reset app</button><label>Node ID<input id="node-id" type="number" min="1" max="127" required></label><button id="assign-node" class="secondary" type="submit">Assign & persist</button><label>CAN baud rate<select id="baud"><option value="0">1 Mbit/s</option><option value="1">800 kbit/s</option><option value="2">500 kbit/s</option><option value="3">250 kbit/s</option><option value="4">125 kbit/s</option><option value="5">100 kbit/s</option><option value="6">50 kbit/s</option></select></label><button id="set-baud" class="secondary" type="button">Set baud rate</button></form>
            </article>
            <article class="pdo-card"><div class="panel-heading"><div><h3>Transmit PDOs</h3><p>Message identifiers and state</p></div><button id="read-pdos" class="small">Read from device</button></div><form id="pdo-form"><div id="pdo-list"></div><button id="apply-pdos" class="primary" type="submit">Apply PDO configuration</button></form></article>
          </div>
        </section>
      </div>
    </div>
  </main>
  <div id="toast" role="status"></div>`;

const $ = (selector) => document.querySelector(selector);
const escapeHtml = (s) => String(s).replace(/[&<>'"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;',"'":'&#39;','"':'&quot;'}[c]));
const formatId = (id, extended) => `0x${Number(id).toString(16).toUpperCase().padStart(extended ? 8 : 3, '0')}`;

function toast(message, error = false) {
  const el = $('#toast'); el.textContent = message; el.className = error ? 'show error' : 'show';
  clearTimeout(toast.timer); toast.timer = setTimeout(() => el.className = '', 3500);
}

async function call(command, args = {}) {
  try { return await invoke(command, args); }
  catch (error) { toast(String(error), true); throw error; }
}

async function refreshInterfaces() {
  const select = $('#interface'); select.disabled = true;
  try {
    state.interfaces = await call('list_interfaces');
    select.innerHTML = state.interfaces.length ? state.interfaces.map((item, i) => `<option value="${i}">${item.kind === 'usb' ? 'USB' : 'SocketCAN'} · ${escapeHtml(item.label)}</option>`).join('') : '<option>No interfaces found</option>';
  } finally { select.disabled = state.connected; }
}

function renderConnection() {
  const pill = $('#connection-pill');
  pill.className = `pill ${state.connected ? 'online' : 'offline'}`;
  pill.innerHTML = `<i></i>${state.connected ? escapeHtml(state.connection.label) : 'Disconnected'}`;
  $('#connect').textContent = state.connected ? 'Disconnect' : 'Connect';
  $('#connect').className = state.connected ? 'danger' : 'primary';
  $('#interface').disabled = state.connected;
  $('#scan').disabled = !state.connected;
}

function renderDevices() {
  $('#device-count').textContent = state.devices.length;
  $('#devices').innerHTML = state.devices.length ? state.devices.map((d, i) => `<button class="device ${state.selected === i ? 'selected' : ''}" data-device="${i}"><span class="device-icon">${icons.bus}</span><span><strong>${d.is_canshunt ? 'CANShunt' : 'CANopen device'}</strong><small>${d.node_id === 255 ? 'Unconfigured' : `Node ${d.node_id}`} · ${d.nmt_state || 'State unknown'}${d.serial != null ? ` · ${Number(d.serial).toString(16).toUpperCase().padStart(8, '0')}` : ''}</small></span>${d.is_canshunt ? '<em>CS</em>' : ''}</button>`).join('') : '<div class="empty">No devices found.<br>Check the bus and scan again.</div>';
  document.querySelectorAll('[data-device]').forEach(el => el.onclick = () => selectDevice(Number(el.dataset.device)));
}

function selectDevice(index) {
  state.selected = index; const d = state.devices[index]; renderDevices();
  $('#welcome').hidden = true; $('#detail').hidden = false;
  $('#device-name').textContent = d.is_canshunt ? 'CANShunt' : 'CANopen device';
  $('#device-meta').innerHTML = `<span>${d.node_id === 255 ? 'Unconfigured' : `Node ${d.node_id}`}</span>${d.serial != null ? `<span>Serial ${Number(d.serial).toString(16).toUpperCase().padStart(8, '0')}</span>` : ''}`;
  $('#node-id').value = d.node_id === 255 ? '' : d.node_id;
  $('#nmt-state-value').textContent = d.nmt_state || (d.node_id === 255 ? 'Unconfigured' : 'Unknown');
  $('#nmt-status').innerHTML = `<i></i>${escapeHtml(d.nmt_state || (d.node_id === 255 ? 'Unconfigured' : 'Unknown'))}`;
  renderReadings(); renderPdos(d.pdos || defaultPdos());
  updateConfigurationAccess(d);
  if (d.is_canshunt && d.node_id !== 255) readPdos();
}

function updateConfigurationAccess(d) {
  const configured = d.is_canshunt && d.node_id !== 255;
  const editable = configured && d.nmt_state === 'PreOperational';
  $('#nmt-action').textContent = d.nmt_state === 'PreOperational' ? 'Start' : 'Reset app';
  $('#nmt-action').disabled = !configured;
  $('#node-id').readOnly = !editable;
  $('#assign-node').classList.toggle('locked', !editable);
  $('#assign-node').setAttribute('aria-disabled', String(!editable));
  $('#set-baud').disabled = !configured;
  $('#baud').disabled = !configured;
  $('#read-pdos').disabled = !configured;
  $('#pdo-form').querySelectorAll('input').forEach(el => el.disabled = !editable);
  $('#apply-pdos').classList.toggle('locked', !editable);
  $('#apply-pdos').setAttribute('aria-disabled', String(!editable));
}

function defaultPdos() { return ['Current low','Current high','Voltage low','Voltage high'].map((name, i) => ({ name, enabled: true, can_id: 0x200 + i, extended: false })); }
function renderPdos(pdos) {
  $('#pdo-list').innerHTML = pdos.map((p, i) => `<div class="pdo-row"><label class="toggle"><input type="checkbox" data-pdo-enable="${i}" ${p.enabled ? 'checked' : ''}><span></span></label><div><strong>${escapeHtml(p.name)}</strong><small>${i % 2 ? 'Channels 4–7' : 'Channels 0–3'}</small></div><label class="id-input">CAN ID<input data-pdo-id="${i}" value="${formatId(p.can_id, p.extended)}" spellcheck="false"></label><label class="extended"><input type="checkbox" data-pdo-ext="${i}" ${p.extended ? 'checked' : ''}> Extended</label></div>`).join('');
}
function renderReadings() {
  $('#readings').innerHTML = state.readings.map((v, i) => `<div class="table-row"><span><b>${i + 1}</b> Channel ${i}</span><span>${v.current == null ? '—' : `${v.current.toLocaleString()} <small>mA</small>`}</span><span>${v.voltage == null ? '—' : `${(v.voltage / 1000).toLocaleString(undefined, { minimumFractionDigits: 3, maximumFractionDigits: 3 })} <small>V</small>`}</span></div>`).join('');
}

async function readPdos() {
  const d = state.devices[state.selected]; if (!d?.is_canshunt) return;
  try { d.pdos = await call('read_pdos', { nodeId: d.node_id }); renderPdos(d.pdos); updateConfigurationAccess(d); }
  catch (_) { /* command already reports the error */ }
}

$('#refresh').onclick = refreshInterfaces;
$('#connect').onclick = async () => {
  if (state.connected) {
    await call('disconnect'); Object.assign(state, { connected: false, connection: null, devices: [], selected: null });
    $('#detail').hidden = true; $('#welcome').hidden = false; renderDevices(); renderConnection(); return;
  }
  const item = state.interfaces[Number($('#interface').value)]; if (!item) return;
  const directDevice = await call('connect', { descriptor: item });
  state.connected = true; state.connection = item;
  if (directDevice) {
    state.devices = [directDevice];
    renderDevices();
    selectDevice(0);
  }
  renderConnection(); toast(`Connected to ${item.label}`);
};
$('#scan').onclick = async () => {
  const button = $('#scan'); button.disabled = true; button.classList.add('busy');
  try { state.devices = await call('scan_bus'); state.selected = null; $('#detail').hidden = true; $('#welcome').hidden = false; renderDevices(); toast(`Found ${state.devices.length} device${state.devices.length === 1 ? '' : 's'}`); }
  finally { button.disabled = false; button.classList.remove('busy'); }
};
$('#read-pdos').onclick = readPdos;
$('#nmt-action').onclick = async () => {
  const d = state.devices[state.selected];
  const requested = d.nmt_state === 'PreOperational' ? 'Start' : 'ResetApp';
  $('#nmt-action').disabled = true;
  try {
    d.nmt_state = await call('set_nmt_state', { nodeId: d.node_id, nmtState: requested });
    renderDevices(); selectDevice(state.selected); toast(`Device is ${d.nmt_state}`);
  } finally { updateConfigurationAccess(d); }
};
$('#device-form').onsubmit = async (event) => {
  event.preventDefault(); const d = state.devices[state.selected]; const newId = Number($('#node-id').value);
  if (d.nmt_state !== 'PreOperational') return toast('Node ID cannot be changed while the device is running. Reset the application first.', true);
  const identity = { vendor_id: d.vendor_id, product_code: d.product_code, revision: d.revision, serial: d.serial };
  await call('assign_node_id', { identity, newNodeId: newId }); d.node_id = newId; renderDevices(); selectDevice(state.selected); toast(`Node ID changed to ${newId}`);
};
$('#set-baud').onclick = async () => { const d = state.devices[state.selected]; await call('set_baudrate', { nodeId: d.node_id, value: Number($('#baud').value) }); toast('Baud rate written'); };
$('#pdo-form').onsubmit = async (event) => {
  event.preventDefault(); const d = state.devices[state.selected];
  if (d.nmt_state !== 'PreOperational') return toast('PDO configuration cannot be changed while the device is running. Reset the application first.', true);
  const pdos = [0,1,2,3].map(i => ({ name: defaultPdos()[i].name, enabled: $(`[data-pdo-enable="${i}"]`).checked, can_id: Number.parseInt($(`[data-pdo-id="${i}"]`).value, 0), extended: $(`[data-pdo-ext="${i}"]`).checked }));
  if (pdos.some(p => !Number.isInteger(p.can_id) || p.can_id < 0 || p.can_id > (p.extended ? 0x1fffffff : 0x7ff))) return toast('One or more CAN IDs are invalid', true);
  await call('write_pdos', { nodeId: d.node_id, pdos }); d.pdos = pdos; toast('PDO configuration applied');
};
document.querySelectorAll('[data-tab]').forEach(button => button.onclick = () => {
  document.querySelectorAll('[data-tab]').forEach(b => b.classList.toggle('active', b === button));
  document.querySelectorAll('.tab-panel').forEach(panel => panel.hidden = panel.id !== button.dataset.tab);
});

async function pollFrames() {
  if (!state.connected) return;
  try {
    const frames = await invoke('poll_frames');
    const device = state.devices[state.selected];
    const nmtStates = { 0: 'Bootup', 4: 'Stopped', 5: 'Operational', 127: 'PreOperational' };
    for (const frame of frames) {
      if (frame.extended || frame.id < 0x701 || frame.id > 0x77f || !frame.data.length) continue;
      const changedDevice = state.devices.find(item => item.node_id === frame.id - 0x700);
      const observed = nmtStates[frame.data[0]];
      if (!changedDevice || !observed || changedDevice.nmt_state === observed) continue;
      changedDevice.nmt_state = observed;
      renderDevices();
      if (changedDevice === device) {
        $('#nmt-status').innerHTML = `<i></i>${escapeHtml(observed)}`;
        $('#nmt-state-value').textContent = observed;
        updateConfigurationAccess(changedDevice);
      }
    }
    if (!device?.is_canshunt || !device.pdos) return;
    let changed = false;
    for (const frame of frames) {
      const pdoIndex = device.pdos.findIndex(p => p.enabled && p.can_id === frame.id && p.extended === frame.extended);
      if (pdoIndex < 0 || frame.data.length < 8) continue;
      const kind = pdoIndex < 2 ? 'current' : 'voltage';
      const offset = pdoIndex % 2 ? 4 : 0;
      for (let i = 0; i < 4; i++) state.readings[offset + i][kind] = frame.data[i * 2] | (frame.data[i * 2 + 1] << 8);
      changed = true;
    }
    if (changed) renderReadings();
  } catch (_) { /* A foreground command can temporarily own the bus. */ }
}
setInterval(pollFrames, 100);
renderReadings(); refreshInterfaces();
