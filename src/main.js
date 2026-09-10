const { invoke } = window.__TAURI__.core;

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
}

document.getElementById("config-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const config = {
    watch_folder: document.getElementById("cfg-watch-folder").value,
    poll_interval_secs: Number(document.getElementById("cfg-poll-interval").value) || 10,
    postgres_conn_string: document.getElementById("cfg-conn-string").value,
    staging_table: document.getElementById("cfg-staging-table").value || "keyence_scan_log",
  };
  await invoke("save_config", { config });
  const msg = document.getElementById("config-saved-msg");
  msg.classList.remove("hidden");
  setTimeout(() => msg.classList.add("hidden"), 2000);
});

document.getElementById("resync-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const filePath = document.getElementById("resync-path").value.trim();
  if (!filePath) return;
  await invoke("force_resync", { filePath });
  document.getElementById("resync-path").value = "";
});

loadConfig();
refreshStatus();
setInterval(refreshStatus, 3000);
