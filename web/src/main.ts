import '@material/web/button/filled-button.js';
import '@material/web/button/outlined-button.js';
import '@material/web/button/text-button.js';
import '@material/web/button/filled-tonal-button.js';
import '@material/web/iconbutton/icon-button.js';
import '@material/web/checkbox/checkbox.js';
import '@material/web/dialog/dialog.js';
import '@material/web/divider/divider.js';
import '@material/web/progress/linear-progress.js';
import '@material/web/radio/radio.js';
import '@material/web/select/outlined-select.js';
import '@material/web/select/select-option.js';
import '@material/web/switch/switch.js';
import '@material/web/textfield/outlined-text-field.js';
import '@material/web/icon/icon.js';
import { argbFromHex, hexFromArgb, themeFromSourceColor } from '@material/material-color-utilities';
import { CandlestickSeries, ColorType, createChart, LineSeries, type IChartApi, type ISeriesApi, type Time } from 'lightweight-charts';
import './style.css';

type Json = Record<string, unknown>;
type Page = 'market' | 'watchlist' | 'portfolio' | 'analysis' | 'ai' | 'audit' | 'settings';
type Company = { id: number; market_id?: number | null; name: string; risk_level?: number | null; status?: string | null; latest_price?: number | null; change_pct?: number | null; available_shares?: number | null };
type Kline = { open: number; high: number; low: number; close: number; volume?: number; time?: number | null; bucket?: string | null };
type Holding = { company_name: string; shares: number; total_value: number; market_id?: number | null; cost?: number; pnl?: number; pnl_pct?: number };
type Portfolio = { balance: number; holdings: Holding[]; total_assets: number; tracked_cost: number; unrealized_pnl: number; realized_pnl: number; captured_at: string };
type Status = { state?: string; minecraft_connected?: boolean; player_name?: string | null; last_market_sync?: string | null; ai_enabled?: boolean; automation_enabled?: boolean; circuit_breaker?: boolean; auth_state?: string };
type Audit = { id?: number; kind?: string; source?: string; status?: string; message: string; created_at?: string; };
type EventItem = { kind?: string; message?: string; at?: string; data?: Json };

const root = document.querySelector<HTMLDivElement>('#app')!;
const api = {
  async request<T>(path: string, init?: RequestInit): Promise<T> {
    const response = await fetch(path, { ...init, headers: { Accept: 'application/json', ...(init?.body ? { 'Content-Type': 'application/json' } : {}), ...init?.headers } });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(typeof body?.error === 'string' ? body.error : `请求失败（${response.status}）`);
    return body as T;
  },
  status: () => api.request<Status>('/api/status'),
  authStatus: () => api.request<Json>('/api/auth/status'),
  companies: () => api.request<Company[]>('/api/market/companies'),
  health: () => api.request<Json>('/api/market/health'),
  kline: (id: number, interval: string) => api.request<Kline[]>(`/api/market/kline/${encodeURIComponent(id)}?interval=${encodeURIComponent(interval)}`),
  available: (id: number, interval: string) => api.request<unknown>(`/api/market/available-shares/${encodeURIComponent(id)}?interval=${encodeURIComponent(interval)}`),
  watchlist: () => api.request<number[]>('/api/watchlist'),
  setWatch: (id: number, enabled: boolean) => api.request<Json>('/api/watchlist', { method: 'POST', body: JSON.stringify({ company_id: id, enabled }) }),
  portfolio: () => api.request<Portfolio>('/api/portfolio'),
  history: () => api.request<unknown[]>('/api/portfolio/history'),
  refreshPortfolio: () => api.request<Portfolio>('/api/portfolio/refresh', { method: 'POST' }),
  audit: () => api.request<Audit[]>('/api/audit'),
  config: () => api.request<Json>('/api/config'),
  aiCapital: () => api.request<Json>('/api/ai/capital'),
  webAuthStatus: () => api.request<Json>('/api/web-auth/status'),
  webAuthSetup: (password: string, confirmation: string) => api.request<Json>('/api/web-auth/setup', { method: 'POST', body: JSON.stringify({ password, confirmation }) }),
  webAuthLogin: (password: string) => api.request<Json>('/api/web-auth/login', { method: 'POST', body: JSON.stringify({ password, confirmation: password }) }),
  webAuthLogout: () => api.request<Json>('/api/web-auth/logout', { method: 'POST' }),
  webAuthPassword: (password: string, confirmation: string) => api.request<Json>('/api/web-auth/password', { method: 'PUT', body: JSON.stringify({ password, confirmation }) }),
  saveConfig: (value: Json) => api.request<Json>('/api/config', { method: 'PUT', body: JSON.stringify(value) }),
  saveSecrets: (value: Json) => api.request<Json>('/api/secrets', { method: 'PUT', body: JSON.stringify(value) }),
};

const money = (value: unknown, digits = 2) => typeof value === 'number' && Number.isFinite(value) ? value.toLocaleString('zh-CN', { minimumFractionDigits: digits, maximumFractionDigits: digits }) : '--';
const percent = (value: unknown) => typeof value === 'number' && Number.isFinite(value) ? `${value >= 0 ? '+' : ''}${value.toFixed(2)}%` : '--';
const time = (value?: string | null) => value ? new Date(value).toLocaleString('zh-CN', { dateStyle: 'short', timeStyle: 'short' }) : '--';
const positive = (value: unknown) => typeof value === 'number' && value >= 0;
const text = (value: unknown, fallback = '--') => value === null || value === undefined || value === '' ? fallback : String(value);

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className?: string, label?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (label !== undefined) node.textContent = label;
  return node;
}
function md(tag: string, label?: string, attrs: Record<string, string> = {}): HTMLElement {
  const node = document.createElement(tag) as HTMLElement;
  Object.entries(attrs).forEach(([key, value]) => node.setAttribute(key, value));
  if (label !== undefined) node.textContent = label;
  return node;
}
const iconPaths: Record<string, string[]> = {
  brightness_6: ['<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.93 4.93l1.42 1.42m11.3 11.3 1.42 1.42M2 12h2m16 0h2M4.93 19.07l1.42-1.42m11.3-11.3 1.42-1.42"/>'],
  refresh: ['<path d="M20 11a8.1 8.1 0 0 0-14.8-4L3 10"/><path d="M3 4v6h6"/><path d="M4 13a8.1 8.1 0 0 0 14.8 4L21 14"/><path d="M21 20v-6h-6"/>'],
  settings: ['<path d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z"/><path d="m19.4 15 .1.1a2 2 0 0 1-2.8 2.8l-.1-.1a2 2 0 0 0-3.4 1.4v.2a2 2 0 0 1-4 0v-.2a2 2 0 0 0-3.4-1.4l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1A2 2 0 0 0 1.6 12a2 2 0 0 0-1.4-3.4H0a2 2 0 0 1 0-4h.2A2 2 0 0 0 1.6 1.2l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1A2 2 0 0 0 7.8-.2V-.4a2 2 0 0 1 4 0v.2a2 2 0 0 0 3.4 1.4l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1A2 2 0 0 0 19.4 7h.2a2 2 0 0 1 0 4h-.2a2 2 0 0 0 0 4Z" transform="translate(0 3) scale(.75)"/>'],
  candlestick_chart: ['<path d="M5 19V9m0 0V5m0 4h3M12 19V5m0 5h3m4 9V8m0 0V3m0 5h3"/><path d="M3 19h18"/>'],
  star: ['<path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-2.9-5.6 2.9 1.1-6.2L3 9.6l6.2-.9L12 3Z"/>'],
  star_border: ['<path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-2.9-5.6 2.9 1.1-6.2L3 9.6l6.2-.9L12 3Z"/>'],
  account_balance_wallet: ['<path d="M4 6.5A2.5 2.5 0 0 1 6.5 4H20v4H6.5A2.5 2.5 0 0 0 4 10.5v7A2.5 2.5 0 0 0 6.5 20H20V9H6.5A2.5 2.5 0 0 1 4 6.5Z"/><path d="M20 13h-4a2 2 0 0 0 0 4h4"/><circle cx="16" cy="15" r=".5"/>'],
  insights: ['<path d="M4 19V5m0 14h16"/><path d="m6 15 4-5 3 3 5-7"/>'],
  smart_toy: ['<rect x="5" y="7" width="14" height="12" rx="3"/><path d="M12 3v4M8 12h.01M16 12h.01M9 16h6"/>'],
  fact_check: ['<path d="M6 4h12v16H6z"/><path d="m9 9 1.5 1.5L13 8m-4 6 1.5 1.5L13 13"/>'],
  data_exploration: ['<path d="M4 19V5m0 14h16"/><path d="m7 15 3-4 3 2 4-6"/><circle cx="17" cy="7" r="1"/>'],
  error_outline: ['<circle cx="12" cy="12" r="9"/><path d="M12 7v5m0 3v.01"/>'],
};
function icon(name: string): SVGSVGElement {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.classList.add('icon-glyph');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  svg.innerHTML = (iconPaths[name] ?? ['<circle cx="12" cy="12" r="8"/>']).join('');
  return svg;
}
function append(parent: ParentNode, ...children: (Node | null | undefined)[]) { children.filter(Boolean).forEach((child) => parent.appendChild(child!)); }
function clear(node: Node) { while (node.firstChild) node.removeChild(node.firstChild); }
function tone(value: unknown): string { return positive(value) ? 'gain' : 'loss'; }

function generateTokens() {
  const dark = themeFromSourceColor(argbFromHex('#49BCE8')).schemes.dark;
  const light = themeFromSourceColor(argbFromHex('#0783B5')).schemes.light;
  document.documentElement.style.setProperty('--md3-generated-primary-dark', hexFromArgb(dark.primary));
  document.documentElement.style.setProperty('--md3-generated-primary-light', hexFromArgb(light.primary));
}

generateTokens();
root.innerHTML = `
  <header class="app-bar">
    <div class="app-brand"><span class="brand-mark">B</span><div><strong>Bilicraft</strong><span>AI Stock</span></div></div>
    <div class="app-actions"><span class="sync-label" id="sync-label">同步状态：--</span><md-icon-button id="theme-toggle" aria-label="切换主题"><span class="icon-slot" data-icon="brightness_6"></span></md-icon-button><md-icon-button id="refresh-all" aria-label="刷新数据"><span class="icon-slot" data-icon="refresh"></span></md-icon-button></div>
  </header>
  <div class="app-layout">
    <aside class="rail" aria-label="主导航"><nav id="desktop-nav"></nav><div class="rail-footer"><md-icon-button id="settings-shortcut" aria-label="设置"><span class="icon-slot" data-icon="settings"></span></md-icon-button><span class="version">v0.1</span></div></aside>
    <main class="page" id="page" tabindex="-1"></main>
  </div>
  <nav class="mobile-nav" id="mobile-nav" aria-label="移动端导航"></nav>
  <div id="toast-region" aria-live="polite"></div>
  <md-dialog id="trade-dialog"><div slot="headline" id="trade-title">交易</div><div slot="content" id="trade-content"></div><div slot="actions"><md-text-button id="trade-cancel">取消</md-text-button><md-filled-button id="trade-submit">提交交易</md-filled-button></div></md-dialog>
`;
root.querySelectorAll<HTMLElement>('[data-icon]').forEach((slot) => slot.replaceWith(icon(slot.dataset.icon || '')));

const pageNode = document.querySelector<HTMLElement>('#page')!;
const syncLabel = document.querySelector<HTMLElement>('#sync-label')!;
const state = { page: (location.hash.slice(1) as Page) || 'market', theme: localStorage.getItem('bilicraft-theme') || 'system', status: {} as Status, companies: [] as Company[], watchlist: [] as number[], portfolio: null as Portfolio | null, kline: [] as Kline[], selectedCompany: null as Company | null, interval: '15m', events: [] as EventItem[], chart: null as IChartApi | null, sharesChart: null as IChartApi | null, candle: null as ISeriesApi<'Candlestick'> | null, sharesLine: null as ISeriesApi<'Line'> | null, config: null as Json | null, settingsSection: 'minecraft', renderToken: 0 };
const navItems: { id: Page; label: string; glyph: string }[] = [
  { id: 'market', label: '行情', glyph: 'candlestick_chart' }, { id: 'watchlist', label: '自选', glyph: 'star' }, { id: 'portfolio', label: '持仓', glyph: 'account_balance_wallet' }, { id: 'analysis', label: '分析', glyph: 'insights' }, { id: 'ai', label: 'AI', glyph: 'smart_toy' }, { id: 'audit', label: '审计', glyph: 'fact_check' }, { id: 'settings', label: '设置', glyph: 'settings' },
];

function navButton(item: typeof navItems[number]) {
  const button = el('button', 'nav-item'); button.type = 'button'; button.dataset.page = item.id; button.setAttribute('aria-label', item.label); button.append(icon(item.glyph), el('span', '', item.label));
  button.addEventListener('click', () => navigate(item.id)); return button;
}
function renderNav() { const desktop = document.querySelector('#desktop-nav')!; const mobile = document.querySelector('#mobile-nav')!; clear(desktop); clear(mobile); navItems.forEach((item) => { desktop.append(navButton(item)); mobile.append(navButton(item)); }); updateNavSelection(); }
function updateNavSelection() { document.querySelectorAll<HTMLElement>('.nav-item').forEach((button) => { const active = button.dataset.page === state.page; button.classList.toggle('active', active); if (active) button.setAttribute('aria-current', 'page'); else button.removeAttribute('aria-current'); }); }
function navigate(page: Page) { state.page = page; location.hash = page; state.selectedCompany = null; updateNavSelection(); renderPage(); pageNode.focus({ preventScroll: true }); }

function surface(title: string, subtitle?: string, actions?: Node[]): HTMLElement { const card = el('section', 'surface'); const head = el('div', 'surface-head'); const copy = el('div'); append(copy, el('h2', '', title), subtitle ? el('p', 'muted', subtitle) : null); append(head, copy); if (actions?.length) { const actionGroup = el('div', 'head-actions'); actions.forEach((action) => actionGroup.append(action)); head.append(actionGroup); } card.append(head); return card; }
function metric(label: string, value: string, detail?: string, className = ''): HTMLElement { const card = el('article', `metric ${className}`); append(card, el('span', 'metric-label', label), el('strong', '', value), detail ? el('span', 'metric-detail', detail) : null); return card; }
function empty(title: string, description: string, action?: HTMLElement): HTMLElement { const node = el('div', 'empty-state'); append(node, icon('data_exploration'), el('h3', '', title), el('p', 'muted', description), action); return node; }
function errorState(message: string, retry?: () => void): HTMLElement { const node = el('div', 'error-state'); append(node, icon('error_outline'), el('strong', '', '数据暂不可用'), el('p', '', message)); if (retry) { const button = md('md-outlined-button', '重试'); button.addEventListener('click', retry); node.append(button); } return node; }
function loading(label = '正在从服务器读取…'): HTMLElement { const node = el('div', 'loading-state'); node.append(md('md-linear-progress')); node.append(el('span', 'muted', label)); return node; }
function button(tag: string, label: string, onClick: () => void, attrs: Record<string, string> = {}): HTMLElement { const node = md(tag, label, attrs); node.addEventListener('click', onClick); return node; }
function field(label: string, value = '', type = 'text'): HTMLElement { const node = md('md-outlined-text-field', undefined, { label, value, type }) as HTMLElement; return node; }
function inputValue(node: HTMLElement): string { return String((node as HTMLInputElement & { value?: string }).value ?? node.getAttribute('value') ?? ''); }
function toast(message: string, kind = 'info') { const region = document.querySelector('#toast-region')!; const item = el('div', `toast ${kind}`, message); region.append(item); window.setTimeout(() => item.remove(), 4200); }
function setPageLoading(label = '正在读取…') { pageNode.replaceChildren(loading(label)); }
function webAuthGate(mode: 'setup_required' | 'login_required') {
  const page = surface(mode === 'setup_required' ? '设置 Web UI 访问密码' : '登录 Web UI', mode === 'setup_required' ? '首次进入必须设置访问密码，密码只保存为不可逆哈希。' : '请输入 Web UI 访问密码继续。');
  const form = el('div', 'auth-gate-form');
  const password = field('访问密码', '', 'password');
  const confirmation = mode === 'setup_required' ? field('确认访问密码', '', 'password') : null;
  const submit = button('md-filled-button', mode === 'setup_required' ? '设置并进入' : '登录', async () => {
    try {
      if (mode === 'setup_required') await api.webAuthSetup(inputValue(password), inputValue(confirmation!));
      else await api.webAuthLogin(inputValue(password));
      toast(mode === 'setup_required' ? '访问密码已设置' : '登录成功', 'success');
      await startApp();
    } catch (error) { toast((error as Error).message, 'error'); }
  });
  append(form, password, confirmation, submit);
  page.append(form);
  pageNode.replaceChildren(page);
}
async function startApp() {
  const auth = await api.webAuthStatus();
  if (auth.state === 'setup_required' || auth.state === 'login_required') { webAuthGate(auth.state); return; }
  renderNav();
  applyTheme();
  void loadBase().then(renderPage);
  startEvents();
}

function companyCard(company: Company, watched: boolean): HTMLElement {
  const card = el('article', 'company-card'); card.tabIndex = 0; card.setAttribute('role', 'button');
  const changeClass = tone(company.change_pct); const main = el('div', 'company-main');
  const title = el('div', 'company-title'); append(title, el('strong', '', company.name), el('span', 'company-id', company.market_id !== null && company.market_id !== undefined ? `#${company.market_id}` : `ID ${company.id}`));
  const meta = el('div', 'company-meta'); append(meta, el('span', '', text(company.status, '状态未知')), company.risk_level ? el('span', `risk risk-${company.risk_level}`, `风险 ${company.risk_level}`) : null);
  append(main, title, meta);
  const quote = el('div', 'company-quote'); append(quote, el('strong', '', money(company.latest_price)), el('span', changeClass, percent(company.change_pct)));
  const star = md('md-icon-button', undefined, { 'aria-label': watched ? '取消自选' : '加入自选' }); star.append(icon(watched ? 'star' : 'star_border')); star.addEventListener('click', (event) => { event.stopPropagation(); toggleWatch(company); });
  append(card, main, quote, star); const open = () => openCompany(company); card.addEventListener('click', open); card.addEventListener('keydown', (event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); open(); } }); return card;
}
function companyGrid(companies: Company[], watchIds: number[]): HTMLElement { const grid = el('div', 'company-grid'); companies.forEach((company) => grid.append(companyCard(company, watchIds.includes(company.id)))); return grid; }

async function toggleWatch(company: Company) { const enabled = !state.watchlist.includes(company.id); try { await api.setWatch(company.id, enabled); state.watchlist = enabled ? [...state.watchlist, company.id] : state.watchlist.filter((id) => id !== company.id); toast(enabled ? `已加入自选：${company.name}` : `已移除自选：${company.name}`, 'success'); renderPage(); } catch (error) { toast((error as Error).message, 'error'); } }
async function loadBase() { try { const [status, companies, watchlist] = await Promise.all([api.status(), api.companies(), api.watchlist()]); state.status = status; state.companies = companies; state.watchlist = watchlist; syncLabel.textContent = status.last_market_sync ? `同步于 ${time(status.last_market_sync)}` : `服务：${text(status.state, '未知')}`; } catch (error) { state.status = {}; syncLabel.textContent = '服务不可用'; if (state.page === 'market') { clear(pageNode); pageNode.append(errorState((error as Error).message, () => { void loadBase().then(renderPage); })); } } }

async function renderMarket() {
  const token = state.renderToken;
  clear(pageNode);
  const refresh = button('md-filled-button', '刷新行情', () => { void loadBase().then(renderPage); }); const page = surface('市场行情', '实时公司报价与交易入口', [refresh]);
  const health = el('div', 'market-health'); const service = state.status.minecraft_connected ? 'Minecraft 已连接' : 'Minecraft 未连接'; append(health, el('span', state.status.minecraft_connected ? 'connected' : 'disconnected', service), el('span', 'muted', `AI ${state.status.ai_enabled ? '已启用' : '未启用'}`), el('span', 'muted', `自动化 ${state.status.automation_enabled ? '运行中' : '关闭'}`)); try { const marketHealth = await api.health(); health.append(el('span', 'muted', typeof marketHealth.status === 'string' ? marketHealth.status : '市场健康检查已返回')); } catch (error) { health.append(el('span', 'loss', `市场健康检查失败：${(error as Error).message}`)); } page.querySelector('.surface-head')?.append(health);
  const summary = el('div', 'metric-grid'); const changes = state.companies.map((company) => company.change_pct).filter((v): v is number => typeof v === 'number'); const average = changes.length ? changes.reduce((a, b) => a + b, 0) / changes.length : null; append(summary, metric('市场股票', String(state.companies.length), '服务器返回'), metric('平均涨跌', percent(average), changes.length ? `${changes.filter((v) => v > 0).length} 上涨 · ${changes.filter((v) => v < 0).length} 下跌` : '暂无报价', tone(average)), metric('连接状态', service, text(state.status.player_name, '等待连接'))); page.append(summary);
  if (token !== state.renderToken || state.page !== 'market') return;
  if (!state.companies.length) page.append(empty('暂无市场数据', '市场 API 尚未返回公司报价。')); else page.append(companyGrid(state.companies, state.watchlist)); pageNode.replaceChildren(page);
}

function renderWatchlist() { const selected = state.companies.filter((company) => state.watchlist.includes(company.id)); const page = surface('我的自选', '只展示你主动关注的公司'); if (!selected.length) page.append(empty('自选列表为空', '在行情页点击星标，将公司加入这里。', button('md-outlined-button', '浏览行情', () => navigate('market')))); else page.append(companyGrid(selected, state.watchlist)); pageNode.replaceChildren(page); }

function portfolioMetrics(portfolio: Portfolio): HTMLElement { const grid = el('div', 'metric-grid portfolio-metrics'); append(grid, metric('总资产', money(portfolio.total_assets), `现金 ${money(portfolio.balance)}`), metric('未实现盈亏', `${positive(portfolio.unrealized_pnl) ? '+' : ''}${money(portfolio.unrealized_pnl)}`, `跟踪成本 ${money(portfolio.tracked_cost)}`, tone(portfolio.unrealized_pnl)), metric('已实现盈亏', `${positive(portfolio.realized_pnl) ? '+' : ''}${money(portfolio.realized_pnl)}`, `快照于 ${time(portfolio.captured_at)}`, tone(portfolio.realized_pnl))); return grid; }
function holdingsTable(holdings: Holding[]): HTMLElement { const wrap = el('div', 'table-wrap'); const table = el('table'); const head = el('thead'); const row = el('tr'); ['公司', '持有', '市值', '成本', '盈亏'].forEach((label) => row.append(el('th', '', label))); head.append(row); const body = el('tbody'); holdings.forEach((holding) => { const tr = el('tr'); const pnl = holding.pnl; [holding.company_name, `${text(holding.shares, '0')} 股`, money(holding.total_value), holding.cost === undefined ? '--' : money(holding.cost), pnl === undefined ? '--' : `${pnl >= 0 ? '+' : ''}${money(pnl)} (${percent(holding.pnl_pct)})`].forEach((value, index) => { const td = el('td', index === 4 && pnl !== undefined ? tone(pnl) : '', value); tr.append(td); }); body.append(tr); }); table.append(head, body); wrap.append(table); return wrap; }
async function renderPortfolio() { const token = state.renderToken; setPageLoading('正在读取组合…'); try { state.portfolio = await api.portfolio(); } catch (error) { clear(pageNode); pageNode.append(errorState((error as Error).message, () => { void renderPortfolio(); })); return; } if (token !== state.renderToken || state.page !== 'portfolio') return; const refresh = button('md-filled-button', '刷新组合', async () => { try { state.portfolio = await api.refreshPortfolio(); toast('组合已从服务器刷新', 'success'); renderPage(); } catch (error) { toast((error as Error).message, 'error'); } }); const page = surface('投资组合', '资金、持仓与服务器确认的成本记录', [refresh]); page.append(portfolioMetrics(state.portfolio)); const holdingSurface = surface('当前持仓', `${state.portfolio.holdings.length} 个真实持仓`); if (!state.portfolio.holdings.length) holdingSurface.append(empty('当前没有持仓', '组合 API 返回了空持仓。')); else holdingSurface.append(holdingsTable(state.portfolio.holdings)); page.append(holdingSurface); pageNode.replaceChildren(page); }

function historyValues(value: unknown): { time?: string; profit?: number; cumulative?: number }[] { const array = Array.isArray(value) ? value : (value && typeof value === 'object' && Array.isArray((value as Json).data) ? (value as Json).data as unknown[] : []); return array.filter((row): row is Json => Boolean(row && typeof row === 'object')).map((row) => ({ time: typeof row.day === 'string' ? row.day : typeof row.captured_at === 'string' ? row.captured_at : undefined, profit: typeof row.daily_profit === 'number' ? row.daily_profit : typeof row.profit === 'number' ? row.profit : undefined, cumulative: typeof row.cumulative_profit === 'number' ? row.cumulative_profit : undefined })); }
function renderHistoryChart(values: { time?: string; profit?: number; cumulative?: number }[]): HTMLElement { const chartWrap = el('div', 'history-chart'); if (!values.some((value) => value.profit !== undefined || value.cumulative !== undefined)) { chartWrap.append(empty('暂无真实收益快照', '服务器尚未提供 portfolio/history 数据，收益曲线不会使用估算值。')); return chartWrap; } const chart = createChart(chartWrap, { height: 260, layout: { background: { type: ColorType.Solid, color: 'transparent' }, textColor: 'var(--md-sys-color-on-surface-variant)' }, grid: { vertLines: { color: 'var(--md-sys-color-outline-variant)' }, horzLines: { color: 'var(--md-sys-color-outline-variant)' } }, rightPriceScale: { borderVisible: false }, timeScale: { borderVisible: false } }); const series = chart.addSeries(LineSeries, { color: '#49BCE8', lineWidth: 2 }); const data = values.map((value, index) => ({ time: (value.time ? Math.floor(new Date(value.time).getTime() / 1000) : Math.floor(Date.now() / 1000) - (values.length - index) * 86400) as Time, value: value.cumulative ?? value.profit ?? 0 })).sort((a, b) => Number(a.time) - Number(b.time)); series.setData(data); chart.timeScale().fitContent(); state.chart = chart; return chartWrap; }
async function renderAnalysis() { const token = state.renderToken; setPageLoading('正在读取收益快照…'); try { const history = await api.history(); const values = historyValues(history); if (token !== state.renderToken || state.page !== 'analysis') return; const page = surface('收益分析', '仅展示服务端记录的真实快照与成本'); const note = el('div', 'notice'); append(note, icon('info'), el('span', '', '收益日历与曲线不补造数据；若后端未记录成本或快照，会清楚显示为空。')); page.append(note, renderHistoryChart(values)); if (values.length) { const table = el('div', 'analysis-list'); values.slice(-8).reverse().forEach((value) => { const item = el('div', 'analysis-row'); append(item, el('span', '', text(value.time)), el('strong', tone(value.profit), value.profit === undefined ? '--' : `${value.profit >= 0 ? '+' : ''}${money(value.profit)}`), el('span', 'muted', value.cumulative === undefined ? '累计收益未记录' : `累计 ${money(value.cumulative)}`)); table.append(item); }); page.append(table); } pageNode.replaceChildren(page); } catch (error) { clear(pageNode); pageNode.append(errorState((error as Error).message, () => { void renderAnalysis(); })); } }

function normalizeTime(point: Kline, index: number): Time { if (point.time) return (point.time > 10_000_000_000 ? Math.floor(point.time / 1000) : point.time) as Time; if (point.bucket) { const parsed = Date.parse(point.bucket); if (!Number.isNaN(parsed)) return Math.floor(parsed / 1000) as Time; } return Math.floor(Date.now() / 1000) - (1000 - index) * 900 as Time; }
function renderMarketChart(container: HTMLElement, points: Kline[], available: unknown) { clear(container); if (!points.length) { container.append(empty('暂无 K 线', '服务器未返回当前周期的 K 线数据。')); return; } const chartNode = el('div', 'chart-node'); const sharesNode = el('div', 'chart-node shares-chart'); container.append(chartNode, sharesNode); const common = { layout: { background: { type: ColorType.Solid, color: 'transparent' }, textColor: 'var(--md-sys-color-on-surface-variant)' }, grid: { vertLines: { color: 'var(--md-sys-color-outline-variant)' }, horzLines: { color: 'var(--md-sys-color-outline-variant)' } }, rightPriceScale: { borderColor: 'var(--md-sys-color-outline)' }, timeScale: { borderColor: 'var(--md-sys-color-outline)', timeVisible: true } } as const; const chart = createChart(chartNode, { ...common, height: 340 }); const candle = chart.addSeries(CandlestickSeries, { upColor: '#16A36A', downColor: '#E64545', borderVisible: false, wickUpColor: '#16A36A', wickDownColor: '#E64545' }); const candleData = points.map((point, index) => ({ time: normalizeTime(point, index), open: point.open, high: point.high, low: point.low, close: point.close })).sort((a, b) => Number(a.time) - Number(b.time)); candle.setData(candleData); chart.timeScale().fitContent(); const shareChart = createChart(sharesNode, { ...common, height: 150 }); const shareLine = shareChart.addSeries(LineSeries, { color: '#49BCE8', lineWidth: 2, title: '剩余股数' }); const shareRows = Array.isArray(available) ? available : (available && typeof available === 'object' && Array.isArray((available as Json).data) ? (available as Json).data : []); const shares = (shareRows as unknown[]).map((row, index) => { const value = typeof row === 'number' ? row : row && typeof row === 'object' ? Number((row as Json).available_shares ?? (row as Json).shares ?? (row as Json).value) : NaN; return Number.isFinite(value) ? { time: candleData[index]?.time ?? candleData[candleData.length - 1]?.time, value } : null; }).filter((row): row is { time: Time; value: number } => Boolean(row?.time)); if (shares.length) { shareLine.setData(shares); shareChart.timeScale().fitContent(); } else { sharesNode.classList.add('hidden-chart'); sharesNode.append(el('span', 'chart-caption', '可用股数 API 未提供可绘制数据')); } chart.timeScale().subscribeVisibleLogicalRangeChange((range) => { if (range) shareChart.timeScale().setVisibleLogicalRange(range); }); shareChart.timeScale().subscribeVisibleLogicalRangeChange((range) => { if (range) chart.timeScale().setVisibleLogicalRange(range); }); chart.subscribeCrosshairMove((param) => { if (param.time) shareChart.setCrosshairPosition(Number((param.seriesData.get(candle) as { close?: number })?.close ?? 0), param.time, shareLine); else shareChart.clearCrosshairPosition(); }); shareChart.subscribeCrosshairMove((param) => { if (param.time) chart.setCrosshairPosition(Number((param.seriesData.get(shareLine) as { value?: number })?.value ?? 0), param.time, candle); else chart.clearCrosshairPosition(); }); state.chart = chart; state.sharesChart = shareChart; state.candle = candle; state.sharesLine = shareLine; const observer = new ResizeObserver(() => { chart.applyOptions({ width: chartNode.clientWidth }); shareChart.applyOptions({ width: sharesNode.clientWidth }); }); observer.observe(chartNode); }

async function openCompany(company: Company) { state.selectedCompany = company; const page = surface(company.name, `市场 ID ${text(company.market_id === null || company.market_id === undefined ? null : company.market_id, String(company.id))} · ${text(company.status, '状态未知')}`); const back = button('md-text-button', '返回行情', () => navigate('market')); page.querySelector('.surface-head')?.prepend(back); const interval = md('md-outlined-select', undefined, { label: '周期' }) as HTMLElement; ['5m', '15m', '1h', '1d'].forEach((value) => interval.append(md('md-select-option', value, { value }))); interval.addEventListener('change', () => { state.interval = String((interval as HTMLSelectElement).value || '15m'); void openCompany(company); }); const star = button('md-filled-tonal-button', state.watchlist.includes(company.id) ? '已自选' : '加入自选', () => { void toggleWatch(company); }); const headActions = page.querySelector('.head-actions'); if (headActions) append(headActions, interval, star); const chartSurface = surface('K 线与剩余股数', '拖动、滚轮缩放，十字线在两个图表间同步'); chartSurface.append(loading('正在读取 K 线…')); pageNode.replaceChildren(page, chartSurface); try { const [points, available] = await Promise.all([api.kline(company.id, state.interval), api.available(company.id, state.interval)]); state.kline = points; const chartBox = el('div', 'charts'); renderMarketChart(chartBox, points, available); chartSurface.replaceChildren(chartBox); } catch (error) { chartSurface.replaceChildren(errorState((error as Error).message)); } const quote = surface('报价与交易'); const quotes = el('div', 'quote-grid'); [['最新价', money(company.latest_price)], ['涨跌幅', percent(company.change_pct)], ['可用股数', company.available_shares === null || company.available_shares === undefined ? '--' : money(company.available_shares, 0)], ['风险等级', company.risk_level ? `风险 ${company.risk_level}` : '--']].forEach(([label, value]) => { const item = el('div', 'quote-item'); append(item, el('span', 'muted', label), el('strong', tone(company.change_pct), value)); quotes.append(item); }); quote.append(quotes, button('md-filled-button', '打开交易面板', () => openTrade(company))); pageNode.append(quote); }

function openTrade(company: Company) { const dialog = document.querySelector<HTMLElement>('#trade-dialog')!; const title = document.querySelector('#trade-title')!; const content = document.querySelector('#trade-content')!; const submit = document.querySelector<HTMLElement>('#trade-submit')!; title.textContent = `交易 · ${company.name}`; clear(content); const actions = el('div', 'trade-actions'); let action: 'buy' | 'sell' = 'buy'; const buy = button('md-filled-tonal-button', '买入', () => { action = 'buy'; buy.setAttribute('selected', ''); sell.removeAttribute('selected'); }); const sell = button('md-filled-tonal-button', '卖出', () => { action = 'sell'; sell.setAttribute('selected', ''); buy.removeAttribute('selected'); }); const amount = field('数量（股）', '', 'number'); append(actions, buy, sell); append(content, actions, amount); submit.onclick = async () => { const shares = Number(inputValue(amount)); if (!Number.isInteger(shares) || shares <= 0) { toast('请输入大于 0 的整数股数', 'error'); return; } try { await api.request<Json>('/api/trade', { method: 'POST', body: JSON.stringify({ action, company_id: company.id, shares, source: 'manual' }) }); toast('交易请求已提交，等待服务器确认', 'success'); (dialog as HTMLDialogElement & { close: () => void }).close(); } catch (error) { toast((error as Error).message, 'error'); } }; (dialog as HTMLDialogElement & { show: () => void }).show(); }

async function renderAi() {
  const token = state.renderToken;
  setPageLoading('正在读取 AI 资金…');
  try {
    const [config, capital, status] = await Promise.all([api.config(), api.aiCapital(), api.status()]);
    if (token !== state.renderToken || state.page !== 'ai') return;
    state.config = config; state.status = status;
    const page = surface('AI 自动交易', status.automation_enabled ? '运行中 · 自动检查行情并决定买卖' : '已停止 · 点击开始后才会定时运行');
    const metrics = el('div', 'metric-grid');
    append(metrics, metric('AI 当前资金', money(capital.total_capital), '投入本金 + 已实现收益'), metric('AI 已实现收益', money(capital.realized_pnl), '卖出后自动复投', tone(capital.realized_pnl)), metric('可继续买入', money(capital.available_cash), `持仓占用 ${money(capital.invested_cost)} · ${capital.cash_verified ? '已核对账户现金' : '未连接，现金待核对'}`));
    page.append(metrics);
    const form = el('div', 'settings-form');
    const principal = field('投入本金', objectValue(config.risk as Json, ['ai_budget']), 'number');
    const frequency = field('检查间隔（分钟）', String(Number((config.ai as Json).automation_interval_seconds || 900) / 60), 'number');
    const save = async () => {
      const value = Number(inputValue(principal)); const minutes = Number(inputValue(frequency));
      if (!Number.isFinite(value) || value <= 0) throw new Error('请输入大于 0 的本金');
      if (!Number.isFinite(minutes) || minutes < 0.5) throw new Error('检查间隔至少半分钟');
      const patch = { risk: { ai_budget: value }, ai: { automation_interval_seconds: Math.round(minutes * 60) } };
      verifyConfigPatch(await api.saveConfig(patch), patch);
    };
    const actions = el('div', 'button-row');
    const action = (label: string, tag: string, work: () => Promise<unknown>) => button(tag, label, async () => { try { await work(); await renderAi(); } catch (error) { toast((error as Error).message, 'error'); } });
    append(actions, action('保存资金设置', 'md-outlined-button', save), action('开始自动交易', 'md-filled-button', async () => { await save(); await api.request('/api/ai/resume', { method: 'POST' }); toast('AI 自动交易已开始', 'success'); }), action('停止', 'md-outlined-button', async () => { await api.request('/api/ai/pause', { method: 'POST' }); toast('AI 自动交易已停止', 'success'); }), action('运行一次（可能下单）', 'md-text-button', async () => { await save(); await api.saveConfig({ ai: { enabled: true } }); const result = await api.request<Json>('/api/ai/run', { method: 'POST', body: '{}' }); toast(`运行完成：${text(result.status)}`, 'success'); }));
    append(form, principal, frequency, actions);
    page.append(form, el('p', 'muted', status.minecraft_connected ? 'Minecraft 已连接。停止会阻止后续自动运行，已发送给服务器的交易仍会完成。' : 'Minecraft 未连接，请先在设置页登录并连接。'));
    const question = field('给 AI 的问题（运行一次，可能下单）');
    const ask = surface('策略问答');
    append(ask, question, action('提交并运行一次', 'md-outlined-button', async () => { await save(); await api.saveConfig({ ai: { enabled: true } }); await api.request('/api/ai/run', { method: 'POST', body: JSON.stringify({ question: inputValue(question) }) }); toast('运行完成', 'success'); }));
    page.append(advancedSettings('更多操作', ask));
    pageNode.replaceChildren(page);
  } catch (error) { if (token === state.renderToken) pageNode.replaceChildren(errorState((error as Error).message)); }
}

async function renderAudit() { const token = state.renderToken; setPageLoading('正在读取审计记录…'); try { const entries = await api.audit(); if (token !== state.renderToken || state.page !== 'audit') return; const page = surface('交易审计', '服务端记录的 AI、风控、交易与连接事件'); if (!entries.length) page.append(empty('暂无审计记录', '服务器尚未返回任何审计事件。')); else { const list = el('div', 'audit-list'); entries.forEach((entry) => { const item = el('article', 'audit-item'); const top = el('div', 'audit-top'); append(top, el('strong', '', text(entry.kind, '事件')), el('span', `audit-status ${entry.status === 'success' ? 'success' : entry.status === 'rejected' ? 'error' : ''}`, text(entry.status)), el('time', 'muted', time(entry.created_at))); append(item, top, el('p', '', entry.message), el('span', 'muted', `来源：${text(entry.source)}`)); list.append(item); }); page.append(list); } pageNode.replaceChildren(page); } catch (error) { clear(pageNode); pageNode.append(errorState((error as Error).message, () => { void renderAudit(); })); } }

function settingsSectionButton(id: string, label: string, active: boolean) { const item = el('button', `settings-tab ${active ? 'active' : ''}`, label); item.type = 'button'; item.addEventListener('click', () => { state.settingsSection = id; renderSettings(); }); return item; }
function objectValue(obj: Json, path: string[]): string { let current: unknown = obj; path.forEach((key) => { current = current && typeof current === 'object' ? (current as Json)[key] : undefined; }); return current === undefined || current === null ? '' : typeof current === 'object' ? JSON.stringify(current) : String(current); }
function settingsField(label: string, obj: Json, path: string[], type = 'text') { const control = field(label, objectValue(obj, path), type); control.dataset.path = path.join('.'); return control; }
function settingsTextArea(label: string, obj: Json, path: string[]) { const control = settingsField(label, obj, path); control.setAttribute('supporting-text', 'JSON 格式'); control.dataset.json = 'true'; return control; }
function settingsToggle(label: string, obj: Json, path: string[]) { const row = el('label', 'settings-toggle'); const control = md('md-switch', undefined, { 'aria-label': label }) as HTMLElement & { selected?: boolean }; control.selected = objectValue(obj, path) === 'true'; control.dataset.path = path.join('.'); control.dataset.boolean = 'true'; row.append(el('span', '', label), control); return row; }
function advancedSettings(title: string, ...controls: HTMLElement[]): HTMLElement {
  const details = el('details', 'advanced-settings');
  details.append(el('summary', '', title), ...controls);
  return details;
}
function settingsGroup(title: string, ...controls: HTMLElement[]) { const group = el('section', 'settings-group'); group.append(el('h3', '', title), ...controls); return group; }
async function authPanel(form: HTMLElement) {
  const section = el('div', 'auth-panel');
  const title = el('h3', '', 'Microsoft / Minecraft 登录');
  const status = el('p', 'muted', '正在读取登录状态…');
  status.setAttribute('role', 'status'); status.setAttribute('aria-live', 'polite');
  const instructions = el('div', 'device-login');
  const actions = el('div', 'button-row');
  let polling = false;
  let timer: number | undefined;
  let stopped = false;
  const visible = () => section.isConnected && state.page === 'settings' && state.settingsSection === 'minecraft';
  const showCode = (result: Json) => {
    instructions.replaceChildren();
    if (!result.user_code || !result.verification_uri) return;
    const link = el('a', 'login-link', '打开 Microsoft 登录页面');
    link.href = String(result.verification_uri); link.target = '_blank'; link.rel = 'noopener noreferrer';
    const code = el('strong', 'device-code', String(result.user_code));
    instructions.append(el('p', '', '打开登录页面，输入下面的验证码，完成后这里会自动更新：'), link, code);
  };
  const failure = (error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    status.textContent = /timeout|timed out|Cannot reach|fetch|abort/i.test(message)
      ? `登录请求超时或网络不可达：服务器需要能访问微软登录接口。请检查服务器网络、DNS 或代理后重试。详情：${message}`
      : `登录失败：${message}`;
    status.classList.add('loss');
    login.removeAttribute('disabled'); login.textContent = '重试获取设备码';
    polling = false;
  };
  const refresh = async () => {
    try {
      const result = await api.request<Json>('/api/auth/status', { signal: AbortSignal.timeout(10000) });
      if (stopped) return;
      status.classList.remove('loss');
      if (result.state === 'failed') { failure(String(result.error || '请重试')); return; }
      if (result.state === 'authenticated') {
        status.textContent = `已登录：${text(result.player_name)}（${text(result.uuid)}）`;
        instructions.replaceChildren(); login.removeAttribute('disabled'); login.textContent = '重新登录';
        if (polling) { polling = false; toast('Microsoft 登录完成', 'success'); }
      } else if (result.state === 'waiting_for_login' || result.state === 'requesting_code') {
        status.textContent = result.state === 'requesting_code' ? '正在向微软获取设备码…' : '等待你完成 Microsoft 登录…';
        login.setAttribute('disabled', ''); login.textContent = '正在等待登录';
        showCode(result); polling = true;
      } else {
        status.textContent = '尚未登录，请获取设备码';
        login.removeAttribute('disabled'); login.textContent = '获取设备码并登录'; polling = false;
      }
    } catch (error) { failure(error); }
  };
  const poll = () => {
    if (timer !== undefined) window.clearTimeout(timer);
    timer = window.setTimeout(async () => {
      if (!visible()) { stopped = true; return; }
      await refresh(); if (polling) poll();
    }, 3000);
  };
  const login = button('md-filled-button', '获取设备码并登录', async () => {
    login.setAttribute('disabled', ''); login.textContent = '正在获取设备码…';
    status.classList.remove('loss'); status.textContent = '正在向微软获取设备码，请稍候（最多约 25 秒）…';
    instructions.replaceChildren();
    try {
      const result = await api.request<Json>('/api/auth/device', { method: 'POST', signal: AbortSignal.timeout(25000) });
      if (!visible()) return;
      showCode(result); status.textContent = '设备码已生成，等待你完成 Microsoft 登录…';
      login.textContent = '正在等待登录'; polling = true; poll();
    } catch (error) { failure(error); }
  });
  const refreshButton = button('md-outlined-button', '刷新登录状态', async () => { await refresh(); if (polling) poll(); });
  append(actions, login, refreshButton); append(section, title, status, instructions, actions); form.append(section);
  await refresh(); if (polling) poll();
}

function collectFields(container: HTMLElement): Json { const result: Json = {}; container.querySelectorAll<HTMLElement>('[data-path]').forEach((node) => { const path = node.dataset.path!.split('.'); let current = result; path.slice(0, -1).forEach((key) => { current[key] = (current[key] && typeof current[key] === 'object' ? current[key] : {}) as Json; current = current[key] as Json; }); const raw = inputValue(node); let value: unknown = node.dataset.boolean === 'true' ? (node as HTMLElement & { selected?: boolean }).selected === true : node.getAttribute('type') === 'number' ? Number(raw) : raw; if (node.dataset.json === 'true') { try { value = raw.trim() ? JSON.parse(raw) : []; } catch { throw new Error(`${path.join('.')} 必须是有效 JSON`); } } current[path[path.length - 1]] = value; }); return result; }
function configurationPatch(section: string, values: Json): Json {
  const key = section === 'minecraft' ? 'server' : section === 'openai' ? 'ai' : section;
  return { [key]: values };
}
function verifyConfigPatch(actual: Json, expected: Json, prefix = ''): void {
  Object.entries(expected).forEach(([key, value]) => {
    const path = prefix ? `${prefix}.${key}` : key;
    if (value !== null && typeof value === 'object' && !Array.isArray(value)) {
      const saved = actual[key];
      if (!saved || typeof saved !== 'object') throw new Error(`${path} 未保存`);
      verifyConfigPatch(saved as Json, value as Json, path);
    } else if (JSON.stringify(actual[key]) !== JSON.stringify(value)) {
      throw new Error(`${path} 保存结果不一致，请重新读取设置`);
    }
  });
}
function webPasswordPanel(): HTMLElement {
  const group = settingsGroup('Web UI 访问密码', el('p', 'muted', '修改后当前会话会退出，请使用新密码重新登录。'));
  const password = field('新访问密码', '', 'password');
  const confirmation = field('确认新访问密码', '', 'password');
  const save = button('md-outlined-button', '修改访问密码', async () => {
    try { await api.webAuthPassword(inputValue(password), inputValue(confirmation)); events?.close(); toast('访问密码已修改，请重新登录', 'success'); webAuthGate('login_required'); } catch (error) { toast((error as Error).message, 'error'); }
  });
  const logout = button('md-text-button', '退出登录', async () => { await api.webAuthLogout(); events?.close(); webAuthGate('login_required'); });
  group.append(password, confirmation, save, logout);
  return group;
}
async function renderSettings() { const token = state.renderToken; setPageLoading('正在读取配置…'); try { state.config = await api.config(); } catch (error) { state.config = {}; toast((error as Error).message, 'error'); } if (token !== state.renderToken || state.page !== 'settings') return; const page = surface('设置', '所有配置由服务端保存；监听地址、路径修改后需重启服务。'); const layout = el('div', 'settings-layout'); const tabs = el('div', 'settings-tabs'); [['minecraft', 'Minecraft'], ['market', '行情'], ['openai', 'OpenAI'], ['risk', 'AI 资金'], ['qq', 'QQ'], ['web', 'Web 服务'], ['paths', '数据路径']].forEach(([id, label]) => tabs.append(settingsSectionButton(id, label, state.settingsSection === id))); const form = el('div', 'settings-form'); const section = state.settingsSection; const config = state.config || {}; if (state.settingsSection === 'minecraft') { await authPanel(form); if (token !== state.renderToken || state.page !== 'settings') return; form.append(el('h3', '', 'Minecraft 连接')); form.append(settingsField('服务器地址', config.server as Json || {}, ['host']), settingsField('端口', config.server as Json || {}, ['port'], 'number'), settingsField('协议号（0=自动）', config.server as Json || {}, ['protocol'], 'number'), settingsField('玩家名称（可留空，使用登录账号）', config.server as Json || {}, ['player_name']), settingsToggle('启动时自动连接', config.server as Json || {}, ['auto_connect'])); form.append(button('md-outlined-button', state.status.minecraft_connected ? '断开连接' : '连接 Minecraft', async () => { try { await api.request<Json>(state.status.minecraft_connected ? '/api/disconnect' : '/api/connect', { method: 'POST' }); toast('连接请求已发送', 'success'); } catch (error) { toast((error as Error).message, 'error'); } })); } else if (state.settingsSection === 'market') { form.append(el('h3', '', '行情数据')); form.append(settingsField('行情 API 地址', config.market as Json || {}, ['base_url']), settingsField('K 线周期', config.market as Json || {}, ['interval']), settingsField('刷新间隔（秒）', config.market as Json || {}, ['sync_seconds'], 'number')); } else if (state.settingsSection === 'openai') {
  form.append(el('h3', '', 'AI 接口'));
  form.append(settingsField('接口地址', config.ai as Json || {}, ['base_url']), settingsField('模型', config.ai as Json || {}, ['model']), settingsField('API Key（留空保留已保存密钥）', {}, ['ai_api_key'], 'password'));
  form.append(advancedSettings('高级接口设置', settingsToggle('允许使用 AI 接口', config.ai as Json || {}, ['enabled']), settingsField('请求路径', config.ai as Json || {}, ['chat_path']), settingsField('请求超时（秒）', config.ai as Json || {}, ['timeout_seconds'], 'number'), settingsField('重试次数', config.ai as Json || {}, ['retries'], 'number'), settingsField('策略提示词（可选）', config.ai as Json || {}, ['system_prompt'])));
} else if (state.settingsSection === 'risk') {
  form.append(el('h3', '', 'AI 资金'));
  form.append(settingsField('投入 AI 的本金', config.risk as Json || {}, ['ai_budget'], 'number'));
  form.append(el('p', 'muted', '这是累计投入本金，不是单轮额度。AI 卖出赚到的钱会自动加回资金，亏损会扣除。开始和停止都在 AI 页面。'));
  const controls = [settingsField('总资产百分比上限（0=不限）', config.risk as Json || {}, ['ai_budget_percent'], 'number'), settingsField('单笔金额上限（0=不限）', config.risk as Json || {}, ['max_single_trade'], 'number'), settingsField('每日亏损上限（0=不限）', config.risk as Json || {}, ['max_daily_loss'], 'number'), settingsField('每日交易次数（0=不限）', config.risk as Json || {}, ['max_daily_trades'], 'number'), settingsField('额外冷却秒数（0=仅服务器冷却）', config.risk as Json || {}, ['cooldown_seconds'], 'number'), settingsField('持仓公司数量（0=不限）', config.risk as Json || {}, ['max_positions'], 'number'), settingsTextArea('交易时间窗口（[]=全天，UTC）', config.risk as Json || {}, ['trading_windows']), settingsTextArea('自动运行窗口（[]=全天，UTC）', config.risk as Json || {}, ['automation_windows'])];
  const reset = button('md-outlined-button', '取消全部额外限制', () => { controls.forEach(control => { (control as HTMLInputElement).value = control.dataset.json === 'true' ? '[]' : '0'; }); toast('额外限制已取消，点击保存生效'); });
  form.append(advancedSettings('可选限制（默认不限制）', settingsToggle('启用额外限制', config.risk as Json || {}, ['optional_limits_enabled']), reset, ...controls));
} else if (state.settingsSection === 'qq') { form.append(el('h3', '', 'QQ 机器人')); form.append(settingsToggle('启用 QQ', config.qq as Json || {}, ['enabled']), settingsField('API Base URL', config.qq as Json || {}, ['api_base_url']), settingsToggle('Markdown 消息', config.qq as Json || {}, ['markdown']), settingsTextArea('发送目标（JSON）', config.qq as Json || {}, ['targets']), settingsField('App ID（仅提交时发送）', {}, ['qq_app_id']), settingsField('App Secret（仅提交时发送）', {}, ['qq_app_secret'], 'password')); const qqActions = el('div', 'button-row'); qqActions.append(button('md-outlined-button', '获取二维码', async () => { try { await api.request<Json>('/api/qq/setup', { method: 'POST' }); toast('二维码绑定已启动，请查看 QQ 绑定状态', 'success'); } catch (error) { toast((error as Error).message, 'error'); } }), button('md-outlined-button', '测试 QQ', async () => { try { await api.request<Json>('/api/qq/test', { method: 'POST' }); toast('QQ 测试已请求', 'success'); } catch (error) { toast((error as Error).message, 'error'); } })); form.append(qqActions); } else if (state.settingsSection === 'web') { form.append(el('h3', '', 'Web 服务')); form.append(settingsField('监听地址', config.web as Json || {}, ['bind']), settingsField('静态目录', config.web as Json || {}, ['static_dir'])); form.append(el('p', 'muted', '监听地址和静态目录保存后需要重启服务。')); } else { form.append(el('h3', '', '数据路径')); form.append(settingsField('数据目录', config.paths as Json || {}, ['data_dir']), settingsField('凭据文件', config.paths as Json || {}, ['secrets_file']), settingsField('数据库文件', config.paths as Json || {}, ['database_file'])); form.append(el('p', 'muted', '数据路径保存后需要重启服务；凭据文件包含敏感令牌，请保持文件权限为 0600。')); } form.append(webPasswordPanel()); const save = button('md-filled-button', '保存设置', async () => { save.setAttribute('disabled', ''); try { const values = collectFields(form); const secrets: Json = {}; for (const key of ['ai_api_key', 'qq_app_id', 'qq_app_secret']) { if (values[key]) secrets[key] = values[key]; delete values[key]; } const patch = configurationPatch(section, values); const saved = await api.saveConfig(patch); verifyConfigPatch(saved, patch); if (Object.keys(secrets).length) await api.saveSecrets(secrets); const verified = await api.config(); verifyConfigPatch(verified, patch); state.config = verified; toast('设置已保存并验证', 'success'); } catch (error) { toast(`保存失败：${(error as Error).message}`, 'error'); } finally { save.removeAttribute('disabled'); } }); form.append(save); layout.append(tabs, form); page.append(layout); pageNode.replaceChildren(page); }

function renderPage() { state.renderToken += 1; clear(pageNode); updateNavSelection(); state.chart?.remove(); state.sharesChart?.remove(); state.chart = null; state.sharesChart = null; if (state.page === 'market') void renderMarket(); else if (state.page === 'watchlist') renderWatchlist(); else if (state.page === 'portfolio') void renderPortfolio(); else if (state.page === 'analysis') void renderAnalysis(); else if (state.page === 'ai') void renderAi(); else if (state.page === 'audit') void renderAudit(); else void renderSettings(); }

function applyTheme() { const theme = state.theme === 'system' ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : state.theme; document.documentElement.dataset.theme = theme; document.querySelector('#theme-toggle')?.setAttribute('aria-label', theme === 'dark' ? '切换到浅色主题' : '切换到深色主题'); }
document.querySelector('#theme-toggle')?.addEventListener('click', () => { state.theme = state.theme === 'dark' ? 'light' : 'dark'; localStorage.setItem('bilicraft-theme', state.theme); applyTheme(); });
document.querySelector('#settings-shortcut')?.addEventListener('click', () => navigate('settings'));
document.querySelector('#refresh-all')?.addEventListener('click', () => { void loadBase().then(renderPage); });
document.querySelector('#trade-cancel')?.addEventListener('click', () => (document.querySelector('#trade-dialog') as HTMLDialogElement & { close: () => void }).close());
window.addEventListener('hashchange', () => { const page = location.hash.slice(1) as Page; if (navItems.some((item) => item.id === page)) { state.page = page; renderPage(); } });
let events: EventSource | null = null;
function startEvents() {
  events?.close();
  events = new EventSource('/api/events');
  events.onmessage = (message) => { try { const event = JSON.parse(message.data) as EventItem; state.events.unshift(event); state.events = state.events.slice(0, 50); if (event.message) toast(event.message, event.kind === 'error' ? 'error' : 'info'); } catch { /* Ignore malformed event frames. */ } };
  events.onerror = () => { syncLabel.textContent = '事件流断开'; };
}
void startApp().catch((error) => { clear(pageNode); pageNode.append(errorState((error as Error).message)); });
