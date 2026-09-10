const { invoke } = window.__TAURI__.core;
const { open: openDialog } = window.__TAURI__.dialog;
const { readTextFile } = window.__TAURI__.fs;

const dbStatusEl = document.getElementById("db-status");
const lastMoEl = document.getElementById("last-mo");
const sentTotalEl = document.getElementById("sent-total");
const lastPollEl = document.getElementById("last-poll");
const errorBannerEl = document.getElementById("error-banner");

async function refreshStatus() {
  try {
    const status = await invoke("get_status");
    dbStatusEl.textContent = status.db_connected ? "OK" : "Chưa kết nối";
    dbStatusEl.className = "value " + (status.db_connected ? "ok" : "warn");
    lastMoEl.textContent = status.last_mo ?? "—";
    sentTotalEl.textContent = status.records_sent_total;
    lastPollEl.textContent = status.last_poll_at ?? "—";

    const err = status.last_db_error || status.last_parse_error;
    if (err) {
      errorBannerEl.textContent = err;
      errorBannerEl.classList.remove("hidden");
    } else {
      errorBannerEl.classList.add("hidden");
    }
  } catch (e) {
    errorBannerEl.textContent = "Không lấy được trạng thái: " + e;
    errorBannerEl.classList.remove("hidden");
  }
}

async function loadConfig() {
  const config = await invoke("get_config");
  document.getElementById("cfg-watch-folder").value = config.watch_folder;
  document.getElementById("cfg-poll-interval").value = config.poll_interval_secs;
  document.getElementById("cfg-conn-string").value = config.postgres_conn_string;
  document.getElementById("cfg-staging-table").value = config.staging_table;
  document.getElementById("cfg-update-server-url").value = config.update_server_url;
}

document.getElementById("config-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const config = {
    watch_folder: document.getElementById("cfg-watch-folder").value,
    poll_interval_secs: Number(document.getElementById("cfg-poll-interval").value) || 10,
    postgres_conn_string: document.getElementById("cfg-conn-string").value,
    staging_table: document.getElementById("cfg-staging-table").value || "keyence_scan_log",
    update_server_url: document.getElementById("cfg-update-server-url").value.trim(),
  };
  await invoke("save_config", { config });
  const msg = document.getElementById("config-saved-msg");
  msg.classList.remove("hidden");
  setTimeout(() => msg.classList.add("hidden"), 2000);
});

document.getElementById("pick-watch-folder").addEventListener("click", async () => {
  const folderInput = document.getElementById("cfg-watch-folder");
  const selected = await openDialog({
    directory: true,
    multiple: false,
    defaultPath: folderInput.value || undefined,
  });
  if (typeof selected === "string") {
    folderInput.value = selected;
  }
});

document.getElementById("pick-resync-path").addEventListener("click", async () => {
  const pathInput = document.getElementById("resync-path");
  const selected = await openDialog({
    directory: false,
    multiple: false,
    defaultPath: pathInput.value || undefined,
  });
  if (typeof selected === "string") {
    pathInput.value = selected;
  }
});

document.getElementById("pick-conn-string-file").addEventListener("click", async () => {
  const connInput = document.getElementById("cfg-conn-string");
  const importErrorEl = document.getElementById("conn-string-import-error");
  importErrorEl.classList.add("hidden");

  const selected = await openDialog({ directory: false, multiple: false });
  if (typeof selected !== "string") return;

  try {
    const content = await readTextFile(selected);
    // TextDecoder mặc định fatal:false -- file sai encoding (vd Notepad lưu
    // "Unicode" = UTF-16LE) KHÔNG throw mà âm thầm thay byte hỏng bằng U+FFFD.
    // Phải tự kiểm, nếu không sẽ nhét connection string rác vào input mà
    // không có cảnh báo gì.
    if (content.includes("�")) {
      importErrorEl.textContent =
        "File có vẻ không phải text UTF-8 hợp lệ (có thể lưu bằng encoding khác) — vui lòng lưu lại bằng UTF-8 rồi thử lại.";
      importErrorEl.classList.remove("hidden");
      return;
    }
    // Chấp nhận file mỗi tham số 1 dòng (dễ đọc/dễ chú thích khi IT chuẩn bị
    // sẵn cho nhiều máy) -- bỏ dòng trống và dòng comment ("#...") trước khi
    // nối lại thành 1 dòng key=value cách nhau bằng khoảng trắng, đúng format
    // driver PostgreSQL cần.
    connInput.value = content
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line !== "" && !line.startsWith("#"))
      .join(" ");
  } catch (e) {
    importErrorEl.textContent = "Không đọc được file: " + e;
    importErrorEl.classList.remove("hidden");
  }
});

document.getElementById("resync-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const filePath = document.getElementById("resync-path").value.trim();
  if (!filePath) return;
  await invoke("force_resync", { filePath });
  document.getElementById("resync-path").value = "";
});

document.querySelectorAll(".card-toggle").forEach((toggle) => {
  const card = toggle.closest(".card");
  const storageKey = "card-collapsed:" + toggle.textContent.trim();
  // Chưa từng bấm (không có key) -> giữ nguyên default đã đặt sẵn trong HTML.
  const stored = localStorage.getItem(storageKey);
  if (stored === "1") {
    card.classList.add("collapsed");
  } else if (stored === "0") {
    card.classList.remove("collapsed");
  }
  toggle.addEventListener("click", () => {
    card.classList.toggle("collapsed");
    localStorage.setItem(storageKey, card.classList.contains("collapsed") ? "1" : "0");
  });
});

loadConfig();
refreshStatus();
setInterval(refreshStatus, 3000);
