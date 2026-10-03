const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open: openDialog } = window.__TAURI__.dialog;
const { openUrl } = window.__TAURI__.opener;

function uiIcon(name) {
  return `<svg class="ui-icon" aria-hidden="true"><use href="icons.svg#${name}" /></svg>`;
}

const LINUX_DOWNLOAD_URL = "https://www.unrealengine.com/linux?lang=pt-BR";
const isWindows = navigator.userAgent.toLowerCase().includes("windows");
let activeProjectLaunch = null;
let buildLogPath = "";
let buildLogLines = [];
let buildUiFrame = null;
let buildUiReady = null;

function appendBuildLog(line) {
  buildLogLines.push(String(line));
  // Keep the webview responsive; the complete log is preserved on disk.
  if (buildLogLines.length > 2000) buildLogLines.splice(0, buildLogLines.length - 2000);
  if (buildUiFrame === null) buildUiFrame = requestAnimationFrame(() => {
    const log = document.getElementById("project-build-log");
    log.textContent = buildLogLines.join("\n");
    log.scrollTop = log.scrollHeight;
    buildUiFrame = null;
  });
}

function setBuildProgress(percent) {
  const bar = document.getElementById("project-build-progress");
  if (percent == null) bar.removeAttribute("value");
  else bar.value = percent;
}

function initBuildUi() {
  document.getElementById("btn-close-project-build").addEventListener("click", () => {
    if (!activeProjectLaunch) document.getElementById("project-build-modal").hidden = true;
  });
  document.getElementById("btn-show-build-log").addEventListener("click", async () => {
    if (!buildLogPath) return;
    try { await invoke("open_path_in_file_manager", { path: buildLogPath }); }
    catch (err) { showToast(String(err), true); }
  });
  return listen("project-build-progress", ({ payload: p }) => {
    if (p.uproject_path !== activeProjectLaunch) return;
    buildLogPath = p.log_path || "";
    document.getElementById("project-build-log-path").textContent = buildLogPath;
    document.getElementById("project-build-status").textContent =
      p.stage === "failed" ? "Falha na compilação" : p.stage === "compiled" ? "Compilação concluída" :
        (p.percent == null ? "Preparando compilação…" : `Compilando… ${Math.round(p.percent)}% das ações`);
    setBuildProgress(p.percent);
    if (p.line) appendBuildLog(p.line);
  });
}

async function launchProject(proj, engineId, rhiMode) {
  if (activeProjectLaunch) { showToast("Aguarde a compilação/abertura atual.", true); return; }
  activeProjectLaunch = proj.uproject_path;
  const modal = document.getElementById("project-build-modal");
  const close = document.getElementById("btn-close-project-build");
  const showLog = document.getElementById("btn-show-build-log");
  buildLogPath = "";
  buildLogLines = [];
  document.getElementById("project-build-log").textContent = "";
  document.getElementById("project-build-log-path").textContent = "";
  document.getElementById("project-build-title").textContent = `Abrindo ${proj.name}`;
  document.getElementById("project-build-status").textContent = "Verificando projeto e preparando compilação…";
  close.disabled = true;
  showLog.disabled = true;
  setBuildProgress(null);
  modal.hidden = false;
  try {
    // Register the listener before invoking; fast builds must not lose their logs.
    await buildUiReady;
    await invoke("launch_project", { uprojectPath: proj.uproject_path, engineId, rhiMode });
    setBuildProgress(100);
    document.getElementById("project-build-status").textContent = "Editor iniciado";
    showToast(`Iniciando ${proj.name}…`);
    if (!buildLogPath) modal.hidden = true;
  } catch (err) {
    document.getElementById("project-build-status").textContent = "Falha ao compilar/abrir o projeto";
    setBuildProgress(0);
    appendBuildLog(String(err));
    showToast("Não foi possível abrir o projeto. Consulte o log.", true);
  } finally {
    activeProjectLaunch = null;
    close.disabled = false;
    showLog.disabled = !buildLogPath;
  }
}

let engines = [];
let projectDirs = [];
let projects = [];
let detectedIdes = [];
let currentView = "dashboard";
let isDownloadActive = false;
let isDownloadPaused = false;
let currentDownloadDestDir = "";

// Cache em memória de thumbnails para evitar re-leitura do disco e codificação base64 repetida
const thumbnailCache = new Map();

// Função estrita de sanitização contra XSS
function escapeHtml(str) {
  if (str === null || str === undefined) return "";
  return String(str)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

/**
 * Trunca caminhos longos de arquivos preservando o nome da raiz e o arquivo final
 * Ex: "Linux_Unreal_Engine_5.5.0/Engine/Source/Runtime/.../Material.h" -> "Linux_Unreal_Engine_5.5.0/…/Material.h"
 */
function truncateFilePath(path, maxLength = 50) {
  if (!path || typeof path !== "string") return "";
  const clean = path.replace(/^(inflating|extracting|creating):\s*/i, "").trim();
  if (!clean || clean.length <= maxLength) return clean;

  const parts = clean.split("/");
  if (parts.length <= 2) {
    const half = Math.floor((maxLength - 3) / 2);
    return clean.slice(0, half) + "…" + clean.slice(-half);
  }

  const filename = parts.pop();
  const root = parts[0];
  const maxMiddle = maxLength - root.length - filename.length - 4;

  if (maxMiddle >= 4 && parts.length > 1) {
    const parentDir = parts[parts.length - 1];
    if (root.length + parentDir.length + filename.length + 6 <= maxLength) {
      return `${root}/…/${parentDir}/${filename}`;
    }
  }

  if (root.length + filename.length + 3 <= maxLength) {
    return `${root}/…/${filename}`;
  }

  const avail = maxLength - filename.length - 3;
  if (avail > 6) {
    return `${clean.slice(0, avail)}…/${filename}`;
  }

  if (filename.length > maxLength) {
    const half = Math.floor((maxLength - 3) / 2);
    return filename.slice(0, half) + "…" + filename.slice(-half);
  }

  return `…/${filename}`;
}


async function refreshDetectedIdes() {
  try {
    detectedIdes = await invoke("get_available_ides");
  } catch (err) {
    console.error("Erro ao detectar IDEs:", err);
  }
}

function renderIdeOptions(selectedIde) {
  const preferred = selectedIde || localStorage.getItem("preferred_ide") || "code";
  if (!detectedIdes || detectedIdes.length === 0) {
    return `
      <option value="code" ${preferred === "code" ? "selected" : ""}>VS Code</option>
      <option value="rider" ${preferred === "rider" ? "selected" : ""}>Rider</option>
      <option value="clion" ${preferred === "clion" ? "selected" : ""}>CLion</option>
    `;
  }
  return detectedIdes
    .map((ide) => {
      const runnerText = ide.runner ? ` (${ide.runner})` : (!ide.is_available ? " (não instalado)" : "");
      const isSelected = preferred === ide.id ? "selected" : "";
      return `<option value="${escapeHtml(ide.id)}" ${isSelected}>${escapeHtml(ide.name)}${escapeHtml(runnerText)}</option>`;
    })
    .join("");
}

function updateTopDownloadWidgetVisibility() {
  const topActive = document.getElementById("top-download-active");
  const topBtn = document.getElementById("btn-open-epic-downloader");

  if (currentView === "downloads") {
    // Na aba de downloads, a pílula vermelha fica SEMPRE oculta conforme solicitado pelo usuário
    if (topActive) topActive.hidden = true;
    if (topBtn) topBtn.hidden = false;
  } else {
    // Nas outras abas (Home, Engines, Projetos), mostra se houver download ativo
    if (isDownloadActive) {
      if (topActive) topActive.hidden = false;
      if (topBtn) topBtn.hidden = false;
    } else {
      if (topActive) topActive.hidden = true;
      if (topBtn) topBtn.hidden = false;
    }
  }
}

// ---------- Navegação entre views ----------
function switchView(viewName) {
  if (!viewName) return;
  currentView = viewName;
  document.querySelectorAll(".nav-item").forEach((b) => {
    b.classList.toggle("active", b.dataset.view === viewName);
  });
  document.querySelectorAll(".view").forEach((v) => {
    v.classList.toggle("active", v.id === `view-${viewName}`);
  });
  updateTopDownloadWidgetVisibility();
  if (viewName === "downloads") {
    renderSteamGraph();
  } else if (viewName === "vault") {
    refreshVault(false);
  }
}

document.querySelectorAll(".nav-item").forEach((btn) => {
  btn.addEventListener("click", () => {
    const viewName = btn.dataset.view;
    if (viewName) switchView(viewName);
  });
});

// ---------- Toast ----------
let toastTimer = null;
function showToast(message, isError = false) {
  const el = document.getElementById("toast");
  el.textContent = message;
  el.classList.toggle("error", isError);
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { el.hidden = true; }, 4000);
}

function updateSidebarStats() {
  const statsEl = document.getElementById("sidebar-stats");
  if (statsEl) {
    statsEl.textContent = `${engines.length} engine${engines.length === 1 ? "" : "s"} • ${projects.length} projeto${projects.length === 1 ? "" : "s"}`;
  }
  const workspaceStats = document.getElementById("workspace-stats");
  if (workspaceStats) workspaceStats.textContent = `${engines.length} engine${engines.length === 1 ? "" : "s"} · ${projects.length} projeto${projects.length === 1 ? "" : "s"}`;
}

// ---------- Engines ----------
async function refreshEngines() {
  engines = await invoke("list_engines");
  renderEngines();
  updateSidebarStats();
}

function renderEngines() {
  // Installed engines remain accessible in their dedicated tab.
  const grid = document.getElementById("engine-list");
  const empty = document.getElementById("engine-empty");
  if (grid && empty) {
    const downloadCard = document.getElementById("btn-open-epic-downloader");
    grid.querySelectorAll(".engine-card:not(.engine-download-card)").forEach((card) => card.remove());
    if (engines.length === 0) {
      empty.hidden = false;
    } else {
      empty.hidden = true;
      for (const eng of engines) {
        const card = document.createElement("div");
        card.className = "engine-card";
        card.innerHTML = `
          <div class="engine-card-top">
            <span class="engine-version">${escapeHtml(eng.version)}</span>
            <span class="engine-badge ${eng.is_source_build ? "source" : ""}">${eng.is_source_build ? "source build" : "installed build"}</span>
          </div>
          <span class="engine-path">${escapeHtml(eng.path)}</span>
          <div class="engine-card-actions">
            <button class="btn btn-small btn-ghost btn-open-editor" data-id="${escapeHtml(eng.id)}">Abrir Editor</button>
            <button class="btn btn-small btn-ghost btn-remove-engine" style="color: var(--danger);" data-id="${escapeHtml(eng.id)}">Remover</button>
          </div>
        `;

        card.querySelector(".btn-open-editor").addEventListener("click", async () => {
          try {
            await invoke("launch_engine_editor", { engineId: eng.id, rhiMode: "auto" });
            showToast(`Abrindo o editor da UE ${eng.version}…`);
          } catch (err) {
            showToast(String(err), true);
          }
        });
        card.querySelector(".btn-remove-engine").addEventListener("click", async () => {
          await invoke("remove_engine", { id: eng.id });
          await refreshEngines();
          await refreshProjects();
          showToast(`Engine ${eng.version} desvinculada do launcher.`);
        });
        grid.insertBefore(card, downloadCard);
      }
    }
  }
}

function showEngineSourceModal() {
  document.getElementById("engine-source-modal").hidden = false;
}
function hideEngineSourceModal() {
  document.getElementById("engine-source-modal").hidden = true;
}

document.getElementById("btn-add-engine").addEventListener("click", showEngineSourceModal);
document.getElementById("engine-source-cancel").addEventListener("click", hideEngineSourceModal);
document.getElementById("engine-source-modal").addEventListener("click", (e) => {
  if (e.target.id === "engine-source-modal") hideEngineSourceModal();
});

document.getElementById("btn-open-epic-downloader").addEventListener("click", openEpicDownloader);
document.getElementById("choice-download-epic").addEventListener("click", () => {
  hideEngineSourceModal();
  openEpicDownloader();
});
document.getElementById("download-engine-cancel").addEventListener("click", () => {
  document.getElementById("download-engine-modal").hidden = true;
});
document.getElementById("download-engine-modal").addEventListener("click", (e) => {
  if (e.target.id === "download-engine-modal") document.getElementById("download-engine-modal").hidden = true;
});

document.getElementById("choice-already-extracted").addEventListener("click", async () => {
  hideEngineSourceModal();
  const folder = await openDialog({
    directory: true,
    title: "Selecione a pasta onde está a engine (pode ser a pasta raiz ou uma pasta acima dela)",
  });
  if (!folder) return;
  showToast("Procurando a engine dentro da pasta selecionada…");
  try {
    const install = await invoke("add_engine_from_folder", { path: folder, isSourceBuild: false });
    showToast(`Engine ${install.version} registrada com sucesso.`);
    await refreshEngines();
  } catch (err) {
    showToast(String(err), true);
  }
});

document.getElementById("choice-extract-zip").addEventListener("click", async () => {
  hideEngineSourceModal();
  const zipPath = await openDialog({
    multiple: false,
    filters: [{ name: "Unreal Engine (.zip)", extensions: ["zip"] }],
    title: "Selecione o .zip baixado",
  });
  if (!zipPath) return;

  const destDir = await openDialog({
    directory: true,
    title: "Escolha onde extrair a engine",
  });
  if (!destDir) return;

  switchView("downloads");
  const activeCard = document.getElementById("steam-active-card");
  const emptyQueue = document.getElementById("steam-empty-queue");
  const queueCount = document.getElementById("steam-queue-count");
  const cardTitle = document.getElementById("steam-card-title");
  const cardPath = document.getElementById("steam-card-path");
  const statusBadge = document.getElementById("steam-status-badge");
  const statusText = document.getElementById("steam-status-text");
  const cardProgress = document.getElementById("steam-card-progress");
  const cardPercent = document.getElementById("steam-card-percent");
  const cardBytes = document.getElementById("steam-card-bytes");
  const cardSpeed = document.getElementById("steam-card-speed");
  const cardEta = document.getElementById("steam-card-eta");

  if (emptyQueue) emptyQueue.hidden = true;
  if (activeCard) activeCard.hidden = false;
  if (queueCount) queueCount.textContent = "Up Next (1)";

  const zipFilename = zipPath.split("/").pop() || "UnrealEngine.zip";
  if (cardTitle) {
    cardTitle.textContent = zipFilename;
    cardTitle.title = zipFilename;
  }
  if (cardPath) {
    cardPath.textContent = destDir;
    cardPath.title = destDir;
  }
  if (statusBadge) {
    statusBadge.className = "steam-card-status-pill extracting";
    if (statusText) statusText.textContent = "Extracting…";
  }
  if (cardProgress) {
    cardProgress.style.width = "0%";
    cardProgress.className = "steam-progress-fill extracting";
  }
  if (cardPercent) cardPercent.textContent = "0%";
  if (cardBytes) {
    cardBytes.textContent = "Descompactando pacote local…";
    cardBytes.title = `Descompactando em ${destDir}`;
  }
  if (cardSpeed) cardSpeed.textContent = "Gravando no disco";
  if (cardEta) {
    cardEta.textContent = "Iniciando extração…";
    cardEta.title = "Iniciando extração…";
  }

  steamNetSpeed = 0.0;
  steamDiskSpeed = 75.0;
  updateSteamMetricsDisplay();

  try {
    const install = await invoke("extract_engine_zip", { zipPath, destDir });
    showToast(`Engine ${install.version} registrada com sucesso.`);
    await refreshEngines();
    if (statusBadge) {
      statusBadge.className = "steam-card-status-pill completed";
      if (statusText) statusText.textContent = "Completed";
    }
    setTimeout(() => {
      if (activeCard) activeCard.hidden = true;
      if (emptyQueue) emptyQueue.hidden = false;
      if (queueCount) queueCount.textContent = "Up Next (0)";
      steamDiskSpeed = 0.0;
      updateSteamMetricsDisplay();
    }, 5000);
  } catch (err) {
    showToast(String(err), true);
    if (activeCard) activeCard.hidden = true;
    if (emptyQueue) emptyQueue.hidden = false;
    if (queueCount) queueCount.textContent = "Up Next (0)";
    steamDiskSpeed = 0.0;
    updateSteamMetricsDisplay();
  }
});

listen("engine-extract-progress", (event) => {
  const { percent, currentFile } = event.payload;
  const pct = Math.round(percent);
  const cardProgress = document.getElementById("steam-card-progress");
  const cardPercent = document.getElementById("steam-card-percent");
  const cardEta = document.getElementById("steam-card-eta");

  if (cardProgress) cardProgress.style.width = `${percent}%`;
  if (cardPercent) cardPercent.textContent = `${pct}%`;
  if (cardEta && currentFile) {
    const clean = currentFile.replace(/^(inflating|extracting|creating):\s*/i, "").trim();
    cardEta.textContent = truncateFilePath(currentFile, 50);
    cardEta.title = clean;
  }
});

// ---------- Projects ----------
async function refreshProjectDirs() {
  projectDirs = await invoke("list_project_dirs");
  renderWatchedDirs();
}

function renderWatchedDirs() {
  const containers = [
    document.getElementById("watched-dirs"),
  ].filter(Boolean);

  for (const row of containers) {
    row.innerHTML = "";
    if (projectDirs.length === 0) row.textContent = "Nenhuma pasta monitorada ainda.";
    for (const dir of projectDirs) {
      const chip = document.createElement("div");
      chip.className = "chip";
      const span = document.createElement("span");
      span.textContent = dir;
      chip.appendChild(span);

      const removeBtn = document.createElement("button");
      removeBtn.innerHTML = uiIcon("close");
      removeBtn.title = `Parar de monitorar ${dir}`;
      removeBtn.setAttribute("aria-label", `Parar de monitorar ${dir}`);
      removeBtn.addEventListener("click", async () => {
        await invoke("remove_project_dir", { path: dir });
        await refreshProjectDirs();
        await refreshProjects();
      });
      chip.appendChild(removeBtn);
      row.appendChild(chip);
    }
  }
}

async function refreshProjects() {
  projects = await invoke("scan_projects");
  renderProjects();
  updateSidebarStats();
}

function applyThumbnailToCard(card, dataUrl, projName) {
  const cover = card.querySelector(".modern-proj-cover");
  if (!cover) return;
  const placeholder = card.querySelector(".modern-proj-placeholder");
  if (placeholder) placeholder.remove();
  const existingImg = cover.querySelector(".modern-proj-cover-img");
  if (existingImg) {
    existingImg.src = dataUrl;
  } else {
    const img = document.createElement("img");
    img.className = "modern-proj-cover-img";
    img.src = dataUrl;
    img.alt = projName;
    cover.prepend(img);
  }
}

function createModernProjectCard(proj) {
  const card = document.createElement("div");
  card.className = "modern-proj-card";

  const engineOptions = engines
    .map(
      (e) =>
        `<option value="${escapeHtml(e.id)}" ${e.id === proj.matched_engine_id ? "selected" : ""}>Unreal Engine ${escapeHtml(e.version)}</option>`
    )
    .join("");

  const matchedEngine = engines.find((e) => e.id === proj.matched_engine_id);
  const engineBadgeText = matchedEngine ? `UE ${escapeHtml(matchedEngine.version)}` : (engines.length > 0 ? "UE ?" : "Sem Engine");
  const initials = escapeHtml((proj.name || "UE").slice(0, 2).toUpperCase());

  card.innerHTML = `
    <div class="modern-proj-cover">
      <div class="modern-proj-placeholder">
        <img src="assets/arcforge-logo.png" alt="" class="modern-proj-placeholder-watermark" />
        <span class="modern-proj-placeholder-initials">${initials}</span>
      </div>
      <div class="modern-proj-badges">
        <span class="badge-engine">${engineBadgeText}</span>
        <span class="badge-type ${proj.has_source ? "cpp" : "bp"}">${proj.has_source ? "C++ Project" : "Blueprint"}</span>
      </div>
    </div>
    <div class="modern-proj-content">
      <h3 class="modern-proj-title" title="${escapeHtml(proj.name)}">${escapeHtml(proj.name)}</h3>
      <span class="modern-proj-path" title="Clique para abrir pasta no sistema: ${escapeHtml(proj.uproject_path)}">${uiIcon("folder")} ${escapeHtml(proj.project_dir)}</span>

      <div class="modern-proj-selectors">
        <select class="select-dark engine-select" title="Versão da Unreal Engine">
          <option value="">Selecionar Engine…</option>
          ${engineOptions}
        </select>
        <select class="select-dark rhi-select" title="Compatibilidade Gráfica (SM5/SM6)" style="max-width: 110px;">
          <option value="auto" ${(!proj.rhi_mode || proj.rhi_mode === "auto") ? "selected" : ""}>Auto</option>
          <option value="sm5" ${proj.rhi_mode === "sm5" ? "selected" : ""}>SM5</option>
          <option value="sm6" ${proj.rhi_mode === "sm6" ? "selected" : ""}>SM6</option>
        </select>
        ${proj.has_source ? `
          <select class="select-dark ide-select" title="IDE para C++" style="max-width: 140px;">
            ${renderIdeOptions()}
          </select>
        ` : ""}
      </div>

      <div class="modern-proj-footer">
        <button class="btn-open-proj-primary btn-open-project" ${engines.length === 0 ? "disabled" : ""}>
          <span>Iniciar Projeto</span>
          ${uiIcon("play")}
        </button>
        ${proj.has_source ? `
          <button class="btn-proj-icon-action btn-open-ide" title="Abrir projeto na IDE">
            ${uiIcon("code")}
          </button>
        ` : ""}
        <button class="btn-proj-icon-action btn-open-folder" title="Abrir pasta no gerenciador de arquivos">
          ${uiIcon("folder")}
        </button>
        <button class="btn-proj-icon-action danger btn-remove-project" title="Remover da lista monitorada">
          ${uiIcon("close")}
        </button>
      </div>
    </div>
  `;

  // Carregamento otimizado de thumbnail com cache em memória
  if (proj.thumbnail_path) {
    if (thumbnailCache.has(proj.thumbnail_path)) {
      applyThumbnailToCard(card, thumbnailCache.get(proj.thumbnail_path), proj.name);
    } else {
      invoke("read_project_thumbnail", { path: proj.thumbnail_path })
        .then((dataUrl) => {
          thumbnailCache.set(proj.thumbnail_path, dataUrl);
          applyThumbnailToCard(card, dataUrl, proj.name);
        })
        .catch(() => {});
    }
  }


  const select = card.querySelector(".engine-select");
  const openBtn = card.querySelector(".btn-open-project");
  const rhiSelect = card.querySelector(".rhi-select");

  if (rhiSelect) {
    rhiSelect.addEventListener("change", async (e) => {
      const mode = e.target.value;
      try {
        await invoke("set_project_rhi_mode", { uprojectPath: proj.uproject_path, rhiMode: mode });
        proj.rhi_mode = mode;
        const label = mode === "auto" ? "Automático" : mode.toUpperCase();
        showToast(`RHI do projeto alterado para: ${label}`);
      } catch (err) {
        showToast(String(err), true);
      }
    });
  }

  openBtn.addEventListener("click", async () => {
    const engineId = select.value;
    if (!engineId) {
      showToast("Selecione qual engine usar para abrir este projeto.", true);
      return;
    }
    const rhiMode = rhiSelect ? rhiSelect.value : (proj.rhi_mode || "auto");
    await launchProject(proj, engineId, rhiMode);
  });

  if (proj.has_source) {
    const ideSelect = card.querySelector(".ide-select");
    if (ideSelect) {
      ideSelect.addEventListener("change", (e) => {
        localStorage.setItem("preferred_ide", e.target.value);
      });
    }
    card.querySelector(".btn-open-ide")?.addEventListener("click", async () => {
      const selectedId = ideSelect ? ideSelect.value : "code";
      const ideMeta = detectedIdes.find((i) => i.id === selectedId);
      const ideName = ideMeta ? `${ideMeta.name}${ideMeta.runner ? ` (${ideMeta.runner})` : ""}` : selectedId;
      try {
        showToast(`Abrindo projeto no ${ideName}…`);
        await invoke("open_project_in_ide", { projectDir: proj.project_dir, ide: selectedId });
      } catch (err) {
        showToast(String(err), true);
      }
    });
  }

  card.querySelector(".btn-open-folder")?.addEventListener("click", async () => {
    try {
      await invoke("open_path_in_file_manager", { path: proj.project_dir });
    } catch (err) {
      showToast(String(err), true);
    }
  });

  card.querySelector(".modern-proj-path")?.addEventListener("click", async () => {
    try {
      await invoke("open_path_in_file_manager", { path: proj.project_dir });
    } catch (err) {
      showToast(String(err), true);
    }
  });

  card.querySelector(".btn-remove-project")?.addEventListener("click", () => {
    openDeleteProjectModal(proj);
  });

  return card;
}

// ---------- Modal de Remoção / Exclusão de Projeto ----------
let projectPendingDeletion = null;

function openDeleteProjectModal(proj) {
  projectPendingDeletion = proj;
  const modal = document.getElementById("delete-project-modal");
  const nameEl = document.getElementById("delete-project-name");
  const pathEl = document.getElementById("delete-project-path");

  if (nameEl) nameEl.textContent = proj.name;
  if (pathEl) pathEl.textContent = proj.uproject_path;
  if (modal) modal.hidden = false;
}

function closeDeleteProjectModal() {
  projectPendingDeletion = null;
  const modal = document.getElementById("delete-project-modal");
  if (modal) modal.hidden = true;
}

document.getElementById("btn-cancel-delete-project")?.addEventListener("click", closeDeleteProjectModal);

document.getElementById("delete-project-modal")?.addEventListener("click", (e) => {
  if (e.target.id === "delete-project-modal") {
    closeDeleteProjectModal();
  }
});

document.getElementById("btn-choice-remove-only")?.addEventListener("click", async () => {
  if (!projectPendingDeletion) return;
  const proj = projectPendingDeletion;
  closeDeleteProjectModal();
  try {
    await invoke("remove_project", { uprojectPath: proj.uproject_path });
    await refreshProjects();
    showToast(`${proj.name} removido da lista do launcher.`);
  } catch (err) {
    showToast(String(err), true);
  }
});

document.getElementById("btn-choice-delete-disk")?.addEventListener("click", async () => {
  if (!projectPendingDeletion) return;
  const proj = projectPendingDeletion;
  closeDeleteProjectModal();
  try {
    await invoke("delete_project_from_disk", { uprojectPath: proj.uproject_path });
    await refreshProjects();
    showToast(`Pasta e arquivos de ${proj.name} foram enviados para a Lixeira.`);
  } catch (err) {
    showToast(`Erro ao excluir projeto: ${err}`, true);
  }
});

function renderProjects() {
  // 1. Renderiza no Dashboard
  const heroContainer = document.getElementById("dashboard-project-list");
  const heroEmpty = document.getElementById("dashboard-project-empty");
  if (heroContainer && heroEmpty) {
    heroContainer.innerHTML = "";
    if (projects.length === 0) {
      heroEmpty.hidden = false;
    } else {
      heroEmpty.hidden = true;
      for (const proj of projects.slice(0, 2)) {
        heroContainer.appendChild(createModernProjectCard(proj));
      }
    }
  }

  // 2. Renderiza na aba dedicada "Projetos"
  const list = document.getElementById("project-list");
  const empty = document.getElementById("project-empty");
  if (list && empty) {
    list.innerHTML = "";
    if (projects.length === 0) {
      empty.hidden = false;
    } else {
      empty.hidden = true;
      for (const proj of projects) {
        list.appendChild(createModernProjectCard(proj));
      }
    }
  }
}

document.getElementById("btn-add-project-dir").addEventListener("click", async () => {
  const folder = await openDialog({ directory: true, title: "Selecione a pasta onde ficam seus projetos" });
  if (!folder) return;
  await invoke("add_project_dir", { path: folder });
  await refreshProjectDirs();
  await refreshProjects();
});

document.getElementById("btn-rescan").addEventListener("click", async () => {
  await refreshProjects();
  showToast("Lista de projetos atualizada.");
});

// ---------- Compatibilidade de GPU e Shader Model (RHI) ----------
let gpuInfo = null;

async function checkGpuCompatibility() {
  try {
    const settings = await invoke("get_rhi_settings");
    gpuInfo = settings.gpu;

    const banner = document.getElementById("gpu-compatibility-banner");
    const hint = document.getElementById("gpu-hint-rhi");

    if (gpuInfo?.recommends_sm5) {
      if (banner) {
        banner.hidden = false;
        const title = document.getElementById("gpu-compat-title");
        const desc = document.getElementById("gpu-compat-desc");
        if (title) title.textContent = "Compatibilidade gráfica (SM5)";
        if (desc) {
          desc.textContent = gpuInfo.reason || "SM5 recomendado para esta GPU.";
        }
      }
      if (hint) {
        hint.textContent = "(SM5 recomendado)";
      }
      const selectNewRhi = document.getElementById("select-new-proj-rhi");
      if (selectNewRhi) {
        selectNewRhi.value = "auto";
      }
    } else if (banner) {
      banner.hidden = true;
    }
  } catch (err) {
    console.warn("Não foi possível verificar a compatibilidade de GPU:", err);
  }
}

// ---------- Modal de Criação de Novo Projeto ----------
function openCreateProjectModal() {
  const modal = document.getElementById("create-project-modal");
  const selectEngine = document.getElementById("select-new-proj-engine");
  const selectRhi = document.getElementById("select-new-proj-rhi");
  const gpuHint = document.getElementById("gpu-hint-rhi");
  const inputDir = document.getElementById("input-new-proj-dir");
  const inputName = document.getElementById("input-new-proj-name");

  if (!modal) return;

  if (engines.length === 0) {
    showToast("Nenhuma Unreal Engine instalada foi encontrada. Adicione ou instale uma engine primeiro.", true);
    return;
  }

  // Preenche opções de engines sanitizadas
  if (selectEngine) {
    selectEngine.innerHTML = engines
      .map((e) => `<option value="${escapeHtml(e.id)}">Unreal Engine ${escapeHtml(e.version)} (${escapeHtml(e.path)})</option>`)
      .join("");
  }


  // Define RHI recomendado
  if (selectRhi) {
    selectRhi.value = "auto";
  }
  if (gpuHint) {
    gpuHint.textContent = "(SM5 recomendado)";
  }

  // Sugere diretório padrão
  if (inputDir) {
    const defaultDir = projectDirs.length > 0 ? projectDirs[0] : "";
    inputDir.value = defaultDir;
  }

  if (inputName) {
    inputName.value = "";
    setTimeout(() => inputName.focus(), 80);
  }

  // Reseta seleção para Blueprint por padrão
  setProjectTypeSelection("blueprint");

  modal.hidden = false;
}

function closeCreateProjectModal() {
  const modal = document.getElementById("create-project-modal");
  if (modal) modal.hidden = true;
}

function setProjectTypeSelection(type) {
  const bpCard = document.getElementById("card-type-bp");
  const cppCard = document.getElementById("card-type-cpp");
  const bpRadio = bpCard?.querySelector("input[type='radio']");
  const cppRadio = cppCard?.querySelector("input[type='radio']");

  if (type === "cpp") {
    bpCard?.classList.remove("active");
    cppCard?.classList.add("active");
    if (cppRadio) cppRadio.checked = true;
  } else {
    cppCard?.classList.remove("active");
    bpCard?.classList.add("active");
    if (bpRadio) bpRadio.checked = true;
  }
}

document.getElementById("card-type-bp")?.addEventListener("click", () => setProjectTypeSelection("blueprint"));
document.getElementById("card-type-cpp")?.addEventListener("click", () => setProjectTypeSelection("cpp"));

document.getElementById("btn-create-project-trigger")?.addEventListener("click", openCreateProjectModal);
document.getElementById("btn-dash-create-project")?.addEventListener("click", openCreateProjectModal);
document.getElementById("btn-hero-create-project")?.addEventListener("click", openCreateProjectModal);
document.getElementById("btn-hero-view-projects")?.addEventListener("click", () => switchView("projects"));
document.getElementById("btn-dashboard-all-projects")?.addEventListener("click", () => switchView("projects"));
document.getElementById("btn-dash-empty-create-project")?.addEventListener("click", openCreateProjectModal);
document.getElementById("btn-projects-empty-create")?.addEventListener("click", openCreateProjectModal);

document.getElementById("btn-close-create-proj-modal")?.addEventListener("click", closeCreateProjectModal);
document.getElementById("btn-cancel-create-project")?.addEventListener("click", closeCreateProjectModal);
document.getElementById("create-project-modal")?.addEventListener("click", (e) => {
  if (e.target.id === "create-project-modal") closeCreateProjectModal();
});

document.getElementById("btn-browse-new-proj-dir")?.addEventListener("click", async () => {
  const folder = await openDialog({
    directory: true,
    title: "Selecione a pasta onde o projeto será criado",
  });
  if (folder) {
    document.getElementById("input-new-proj-dir").value = folder;
  }
});

document.getElementById("btn-rescan-projects-tab")?.addEventListener("click", async () => {
  await refreshProjects();
  showToast("Lista de projetos atualizada.");
});

document.getElementById("form-create-project")?.addEventListener("submit", async (e) => {
  e.preventDefault();

  const nameInput = document.getElementById("input-new-proj-name");
  const templateSelect = document.getElementById("select-new-proj-template");
  const engineSelect = document.getElementById("select-new-proj-engine");
  const dirInput = document.getElementById("input-new-proj-dir");
  const submitBtn = document.getElementById("btn-submit-create-project");

  const name = nameInput.value.trim();
  const template = templateSelect.value;
  const engineId = engineSelect.value;
  const parentDir = dirInput.value.trim();
  const isCpp = document.querySelector("input[name='proj-type']:checked")?.value === "cpp";

  if (!name) {
    showToast("Digite o nome do projeto.", true);
    return;
  }
  if (!/^[a-zA-Z][a-zA-Z0-9_]*$/.test(name)) {
    showToast("O nome do projeto deve começar com uma letra e conter apenas letras, números e sublinhados (_).", true);
    return;
  }
  if (!engineId) {
    showToast("Selecione uma engine para associar ao projeto.", true);
    return;
  }
  if (!parentDir) {
    showToast("Selecione a pasta de destino para o projeto.", true);
    return;
  }

  const rhiSelect = document.getElementById("select-new-proj-rhi");
  const rhiTarget = rhiSelect ? rhiSelect.value : "auto";

  const originalBtnHtml = submitBtn.innerHTML;
  submitBtn.disabled = true;
  submitBtn.innerHTML = `<span>Criando projeto… ⏳</span>`;

  try {
    const newProj = await invoke("create_project", {
      name,
      parentDir,
      engineId,
      projectType: isCpp ? "cpp" : "blueprint",
      template,
      rhiTarget,
    });

    if (!projectDirs.includes(parentDir)) {
      await invoke("add_project_dir", { path: parentDir });
      await refreshProjectDirs();
    }

    closeCreateProjectModal();
    showToast(`Projeto ${newProj.name} (${isCpp ? "C++" : "Blueprint"}) criado com sucesso!`);
    await refreshProjects();
  } catch (err) {
    showToast(`Erro ao criar projeto: ${err}`, true);
  } finally {
    submitBtn.disabled = false;
    submitBtn.innerHTML = originalBtnHtml;
  }
});

// ---------- Conta Epic Games (Topbar Perfil) ----------
async function refreshEpicStatus() {
  const generation = vaultGeneration;
  const userDisplayName = document.getElementById("user-display-name");
  const userAvatar = document.getElementById("user-avatar");
  const loginBtn = document.getElementById("btn-epic-login");
  const logoutBtn = document.getElementById("btn-epic-logout");

  try {
    const status = await invoke("epic_status");
    if (generation !== vaultGeneration) return null;
    setVaultAccount(status.logged_in ? status.account_id : null);

    if (status.logged_in) {
      const name = status.username || "Conectado";
      userDisplayName.textContent = name;
      userAvatar.textContent = name.slice(0, 1).toUpperCase();
      loginBtn.hidden = true;
      logoutBtn.hidden = false;
    } else {
      userDisplayName.textContent = "Não conectado";
      userAvatar.textContent = "?";
      loginBtn.hidden = false;
      logoutBtn.hidden = true;
    }
    return status;
  } catch (err) {
    if (generation !== vaultGeneration) return null;
    setVaultAccount(null);
    userDisplayName.textContent = "Não conectado";
    userAvatar.textContent = "?";
    loginBtn.hidden = false;
    logoutBtn.hidden = true;
    return null;
  }
}

document.getElementById("btn-epic-login").addEventListener("click", () => {
  document.getElementById("epic-login-status").textContent = "";
  document.getElementById("epic-login-log").hidden = true;
  document.getElementById("epic-login-log").textContent = "";
  document.getElementById("epic-login-modal").hidden = false;
});

document.getElementById("epic-login-cancel").addEventListener("click", () => {
  document.getElementById("epic-login-modal").hidden = true;
});

document.getElementById("epic-login-modal").addEventListener("click", (e) => {
  if (e.target.id === "epic-login-modal") document.getElementById("epic-login-modal").hidden = true;
});

document.getElementById("btn-open-epic-login").addEventListener("click", async () => {
  const statusEl = document.getElementById("epic-login-status");
  const log = document.getElementById("epic-login-log");
  statusEl.textContent = "Aguardando você logar na janela que abriu…";
  log.hidden = false;
  log.textContent = "";
  try {
    await invoke("epic_login_auto");
    showToast("Login com a conta Epic concluído com sucesso!");
    document.getElementById("epic-login-modal").hidden = true;
  } catch (err) {
    statusEl.textContent = "Não foi possível concluir o login automaticamente.";
    showToast(String(err), true);
  } finally {
    await refreshEpicStatus();
    refreshVault(true).catch(() => {});
  }
});

document.getElementById("btn-epic-logout").addEventListener("click", async () => {
  setVaultAccount(null, true);
  try {
    await invoke("epic_logout");
    showToast("Você saiu da conta Epic.");
  } catch (err) {
    showToast(String(err), true);
  } finally {
    await refreshEpicStatus();
    renderVaultGrid();
  }
});

function appendEpicLog(line) {
  const log = document.getElementById("epic-login-log");
  if (!log) return;
  log.textContent += line + "\n";
  log.scrollTop = log.scrollHeight;
}

listen("epic-log", (event) => appendEpicLog(event.payload.line));
listen("legendary-log", (event) => appendEpicLog(event.payload.line));

// ---------- Catálogo e Download Nativo de Engines (Cosmos API & SSO) ----------
let currentAvailableBlobs = [];
let currentCategoryFilter = "all";
let currentSearchFilter = "";

function renderAvailableEngines() {
  const modal = document.getElementById("download-engine-modal");
  const listEl = document.getElementById("available-engines-list");
  listEl.innerHTML = "";

  const query = currentSearchFilter.trim().toLowerCase();
  const filtered = currentAvailableBlobs.filter((blob) => {
    const ver = (blob.version || blob.name || "").toLowerCase();
    const matchesQuery = !query || ver.includes(query) || blob.name.toLowerCase().includes(query);
    if (!matchesQuery) return false;

    if (currentCategoryFilter === "5") {
      return ver.startsWith("5.");
    } else if (currentCategoryFilter === "4") {
      return ver.startsWith("4.");
    }
    return true;
  });

  if (filtered.length === 0) {
    listEl.innerHTML = `<div class="empty-state" style="margin: 20px 0;"><p>Nenhuma versão encontrada para o filtro atual.</p></div>`;
    return;
  }

  for (const blob of filtered) {
    const isPrecompiled = blob.isPrecompiled !== false && (blob.version ? blob.version.startsWith("5.") : true);
    const ver = blob.version || blob.name.replace("Linux_Unreal_Engine_", "").replace(".zip", "");
    const cleanName = `Unreal Engine ${ver}`;
    const sizeGb = blob.size > 0 ? (blob.size / 1e9).toFixed(1) + " GB" : "Variável";
    const isInstalled = engines.some((e) => e.version === ver || cleanName.includes(e.version));

    const card = document.createElement("div");
    card.className = "available-card";

    const badgePrecompiled = isPrecompiled
      ? `<span style="background: rgba(34, 197, 94, 0.15); color: #4ade80; border: 1px solid rgba(34, 197, 94, 0.3); padding: 2px 7px; border-radius: 4px; font-size: 11px;">Pré-compilada Oficial</span>`
      : `<span style="background: rgba(59, 130, 246, 0.15); color: #60a5fa; border: 1px solid rgba(59, 130, 246, 0.3); padding: 2px 7px; border-radius: 4px; font-size: 11px;">Código Fonte (GitHub)</span>`;

    const isWindows = navigator.userAgent.toLowerCase().includes("windows") || (navigator.platform && navigator.platform.toLowerCase().includes("win"));

    const metaText = isPrecompiled
      ? `${sizeGb} • Build oficial da Epic Games para ${isWindows ? "Windows" : "Linux"}`
      : `Repositório oficial da Epic Games • GitHub Release`;

    const actionBtnText = isInstalled
      ? "Reinstalar"
      : (isPrecompiled ? (isWindows ? "Instalar via Epic Games ↗" : "Baixar & Instalar") : "Obter no GitHub ↗");

    card.innerHTML = `
      <div class="available-info">
        <span class="available-title">
          ${escapeHtml(cleanName)}
          ${badgePrecompiled}
          ${isInstalled ? '<span class="engine-badge">instalada</span>' : ""}
        </span>
        <span class="available-meta">${metaText}</span>
      </div>
      <button class="btn btn-small ${isInstalled ? "btn-ghost" : (isPrecompiled ? "btn-primary" : "btn-ghost")} btn-download-blob">
        ${actionBtnText}
      </button>
    `;

    card.querySelector(".btn-download-blob").addEventListener("click", async () => {
      if (!isPrecompiled) {
        try {
          const ghUrl = `https://github.com/EpicGames/UnrealEngine/tree/${ver}-release`;
          await openUrl(ghUrl);
          showToast(`Abrindo repositório oficial da UE ${ver} no GitHub...`);
        } catch (err) {
          showToast(`Erro ao abrir navegador: ${err}`, true);
        }
        return;
      }

      if (isWindows) {
        try {
          await openUrl("com.epicgames.launcher://apps/ue");
          showToast(`Abrindo Epic Games Launcher para gerenciar a ${cleanName}…`);
        } catch {
          await openUrl("https://store.epicgames.com/download");
          showToast("Abra o Epic Games Launcher para instalar versões oficiais no Windows.", true);
        }
        return;
      }

      // 1. Fecha o catálogo e pede imediatamente onde instalar
      modal.hidden = true;
      const destDir = await openDialog({
        directory: true,
        title: `Escolha a pasta onde deseja instalar a ${cleanName}`,
      });
      if (!destDir) {
        modal.hidden = false;
        return;
      }

      // 2. Inicia o download e extração automaticamente
      startEngineDownload(blob, cleanName, destDir);
    });

    listEl.appendChild(card);
  }
}

document.getElementById("engine-search-filter").addEventListener("input", (e) => {
  currentSearchFilter = e.target.value;
  renderAvailableEngines();
});

document.querySelectorAll(".filter-tab").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".filter-tab").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    currentCategoryFilter = btn.dataset.filter;
    renderAvailableEngines();
  });
});

async function loadEngineCatalog(force = false) {
  const loading = document.getElementById("available-engines-loading");
  const errorEl = document.getElementById("available-engines-error");
  const errorMsg = document.getElementById("available-engines-error-msg");
  const listEl = document.getElementById("available-engines-list");
  const refreshBtn = document.getElementById("btn-refresh-catalog");

  if (force && refreshBtn) {
    refreshBtn.disabled = true;
    refreshBtn.innerHTML = `${uiIcon("refresh")} Atualizando…`;
  }

  // Se já tivermos itens em memória ou cache, não bloqueia a tela com loading pesado
  if (currentAvailableBlobs.length === 0) {
    loading.hidden = false;
    errorEl.hidden = true;
    listEl.hidden = true;
  }

  try {
    const blobs = await invoke("epic_list_available_engines", { forceRefresh: force });
    loading.hidden = true;

    if (!blobs || blobs.length === 0) {
      if (currentAvailableBlobs.length === 0) {
        errorEl.hidden = false;
        errorMsg.textContent = "Nenhuma versão encontrada para a conta conectada.";
      }
      return;
    }

    currentAvailableBlobs = blobs;
    errorEl.hidden = true;
    listEl.hidden = false;
    renderAvailableEngines();
    if (force) {
      showToast("Catálogo atualizado!");
    }
  } catch (err) {
    loading.hidden = true;
    if (currentAvailableBlobs.length === 0) {
      errorEl.hidden = false;
      errorMsg.textContent = String(err);
    } else {
      showToast(`Erro ao atualizar catálogo: ${err}`, true);
    }
  } finally {
    if (refreshBtn) {
      refreshBtn.disabled = false;
      refreshBtn.innerHTML = `${uiIcon("refresh")} Atualizar`;
    }
  }
}

// Ouve atualizações em background vindas do worker
listen("engine-catalog-updated", (event) => {
  if (Array.isArray(event.payload) && event.payload.length > 0) {
    currentAvailableBlobs = event.payload;
    const modal = document.getElementById("download-engine-modal");
    if (!modal.hidden) {
      renderAvailableEngines();
      showToast("Catálogo de versões atualizado pelo servidor!");
    }
  }
});

document.getElementById("btn-refresh-catalog").addEventListener("click", () => loadEngineCatalog(true));
document.getElementById("btn-retry-catalog").addEventListener("click", () => loadEngineCatalog(true));

async function openEpicDownloader() {
  const status = await invoke("epic_status");
  if (!status.logged_in) {
    showToast("Faça login com sua conta Epic primeiro para baixar a engine.", true);
    document.getElementById("epic-login-status").textContent = "";
    document.getElementById("epic-login-log").hidden = true;
    document.getElementById("epic-login-log").textContent = "";
    document.getElementById("epic-login-modal").hidden = false;
    return;
  }

  const modal = document.getElementById("download-engine-modal");
  modal.hidden = false;
  await loadEngineCatalog(false);
}

// ---------- Gerenciador de Downloads e Gráfico Estilo Steam ----------
const STEAM_MAX_SAMPLES = 60;
let steamNetSpeed = 0.0;
let steamDiskSpeed = 0.0;
let steamPeakSpeed = 0.0;
let steamGraphSamples = Array.from({ length: STEAM_MAX_SAMPLES }, () => ({ net: 0, disk: 0 }));

function formatSteamSpeed(mbps) {
  if (!mbps || mbps <= 0.01) return "0 bps";
  if (mbps < 0.1) return `${(mbps * 1024).toFixed(1)} KB/s`;
  return `${mbps.toFixed(1)} MB/s`;
}

function updateSteamMetricsDisplay() {
  const netEl = document.getElementById("steam-net-speed");
  const peakEl = document.getElementById("steam-peak-speed");
  const diskEl = document.getElementById("steam-disk-speed");

  if (netEl) netEl.textContent = formatSteamSpeed(steamNetSpeed);
  if (peakEl) peakEl.textContent = formatSteamSpeed(steamPeakSpeed);
  if (diskEl) diskEl.textContent = formatSteamSpeed(steamDiskSpeed);
}

function renderSteamGraph() {
  const canvas = document.getElementById("steam-graph-canvas");
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  const rect = canvas.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  const displayWidth = Math.round(rect.width) || 600;
  const displayHeight = Math.round(rect.height) || 150;

  if (canvas.width !== displayWidth * dpr || canvas.height !== displayHeight * dpr) {
    canvas.width = displayWidth * dpr;
    canvas.height = displayHeight * dpr;
  }

  ctx.save();
  ctx.scale(dpr, dpr);
  const w = displayWidth;
  const h = displayHeight;

  ctx.clearRect(0, 0, w, h);

  // Escala dinâmica baseada na maior velocidade observada
  let maxObserved = 10;
  for (const s of steamGraphSamples) {
    if (s.net > maxObserved) maxObserved = s.net;
    if (s.disk > maxObserved) maxObserved = s.disk;
  }
  const maxVal = Math.max(maxObserved * 1.15, 10);

  // Linhas horizontais sutis e rótulos
  ctx.strokeStyle = "rgba(255, 255, 255, 0.06)";
  ctx.lineWidth = 1;
  const gridLines = 4;
  for (let i = 1; i <= gridLines; i++) {
    const y = h - (h / (gridLines + 1)) * i;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
    ctx.stroke();

    const labelVal = (maxVal / (gridLines + 1)) * i;
    ctx.fillStyle = "rgba(255, 255, 255, 0.28)";
    ctx.font = "9.5px 'Fira Code', monospace";
    ctx.textAlign = "right";
    ctx.fillText(formatSteamSpeed(labelVal), w - 8, y - 4);
  }

  const step = w / (STEAM_MAX_SAMPLES - 1);

  // 1. Curva de Velocidade de Download (Azul / Ciano com preenchimento em degradê)
  ctx.beginPath();
  ctx.moveTo(0, h);
  for (let i = 0; i < STEAM_MAX_SAMPLES; i++) {
    const val = steamGraphSamples[i].net;
    const x = i * step;
    const y = h - (val / maxVal) * (h - 16);
    ctx.lineTo(x, y);
  }
  ctx.lineTo(w, h);
  ctx.closePath();

  const gradNet = ctx.createLinearGradient(0, 0, 0, h);
  gradNet.addColorStop(0, "rgba(56, 189, 248, 0.28)");
  gradNet.addColorStop(1, "rgba(56, 189, 248, 0.0)");
  ctx.fillStyle = gradNet;
  ctx.fill();

  ctx.beginPath();
  for (let i = 0; i < STEAM_MAX_SAMPLES; i++) {
    const val = steamGraphSamples[i].net;
    const x = i * step;
    const y = h - (val / maxVal) * (h - 16);
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.strokeStyle = "#38bdf8";
  ctx.lineWidth = 2;
  ctx.shadowColor = "rgba(56, 189, 248, 0.5)";
  ctx.shadowBlur = 6;
  ctx.stroke();
  ctx.shadowBlur = 0;

  // 2. Curva de Uso de Disco (Verde Neon)
  ctx.beginPath();
  for (let i = 0; i < STEAM_MAX_SAMPLES; i++) {
    const val = steamGraphSamples[i].disk;
    const x = i * step;
    const y = h - (val / maxVal) * (h - 16);
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.strokeStyle = "#4ade80";
  ctx.lineWidth = 1.5;
  ctx.stroke();

  ctx.restore();
}

// Timer para empurrar amostras para o gráfico a cada segundo
setInterval(() => {
  steamGraphSamples.push({ net: steamNetSpeed, disk: steamDiskSpeed });
  if (steamGraphSamples.length > STEAM_MAX_SAMPLES) {
    steamGraphSamples.shift();
  }
  updateSteamMetricsDisplay();

  const dlView = document.getElementById("view-downloads");
  if (dlView && dlView.classList.contains("active")) {
    renderSteamGraph();
  }
}, 1000);

window.addEventListener("resize", () => {
  const dlView = document.getElementById("view-downloads");
  if (dlView && dlView.classList.contains("active")) {
    renderSteamGraph();
  }
});

document.getElementById("btn-steam-browse-catalog")?.addEventListener("click", openEpicDownloader);
document.getElementById("btn-dl-catalog")?.addEventListener("click", openEpicDownloader);
document.getElementById("btn-steam-gear")?.addEventListener("click", openEpicDownloader);

// Controles de Download (Pausar / Retomar / Cancelar / Pasta)
document.getElementById("btn-steam-pause")?.addEventListener("click", async () => {
  const pauseIcon = document.getElementById("btn-steam-pause-icon");
  const pauseText = document.getElementById("btn-steam-pause-text");
  const statusBadge = document.getElementById("steam-status-badge");
  const statusText = document.getElementById("steam-status-text");

  if (!isDownloadPaused) {
    await invoke("epic_pause_download");
    isDownloadPaused = true;
    steamNetSpeed = 0.0;
    steamDiskSpeed = 0.0;
    updateSteamMetricsDisplay();

    if (pauseIcon) pauseIcon.textContent = "▶";
    if (pauseText) pauseText.textContent = "Retomar";
    if (statusBadge) {
      statusBadge.className = "steam-card-status-pill paused";
      if (statusText) statusText.textContent = "Paused";
    }
    showToast("Download pausado.");
  } else {
    await invoke("epic_resume_download");
    isDownloadPaused = false;

    if (pauseIcon) pauseIcon.textContent = "⏸";
    if (pauseText) pauseText.textContent = "Pausar";
    if (statusBadge) {
      statusBadge.className = "steam-card-status-pill";
      if (statusText) statusText.textContent = "Downloading…";
    }
    showToast("Download retomado.");
  }
});

document.getElementById("btn-steam-cancel")?.addEventListener("click", async () => {
  if (confirm("Deseja realmente cancelar este download? Os arquivos temporários serão excluídos.")) {
    try {
      await invoke("epic_cancel_download");
    } catch (e) {}

    isDownloadActive = false;
    isDownloadPaused = false;
    steamNetSpeed = 0.0;
    steamDiskSpeed = 0.0;
    updateSteamMetricsDisplay();

    const activeCard = document.getElementById("steam-active-card");
    const emptyQueue = document.getElementById("steam-empty-queue");
    const queueCount = document.getElementById("steam-queue-count");
    if (activeCard) activeCard.hidden = true;
    if (emptyQueue) emptyQueue.hidden = false;
    if (queueCount) queueCount.textContent = "Up Next (0)";

    updateTopDownloadWidgetVisibility();
    showToast("Download cancelado pelo usuário.");
  }
});

document.getElementById("btn-steam-folder")?.addEventListener("click", async () => {
  if (currentDownloadDestDir) {
    try {
      await invoke("open_path_in_file_manager", { path: currentDownloadDestDir });
    } catch (err) {
      showToast(String(err), true);
    }
  }
});

async function startEngineDownload(blob, cleanName, destDir) {
  // 1. Abre diretamente a view de Downloads estilo Steam
  switchView("downloads");

  isDownloadActive = true;
  isDownloadPaused = false;
  currentDownloadDestDir = destDir;
  updateTopDownloadWidgetVisibility();

  const pauseIcon = document.getElementById("btn-steam-pause-icon");
  const pauseText = document.getElementById("btn-steam-pause-text");
  if (pauseIcon) pauseIcon.textContent = "⏸";
  if (pauseText) pauseText.textContent = "Pausar";

  const activeCard = document.getElementById("steam-active-card");
  const emptyQueue = document.getElementById("steam-empty-queue");
  const queueCount = document.getElementById("steam-queue-count");
  const cardTitle = document.getElementById("steam-card-title");
  const cardPath = document.getElementById("steam-card-path");
  const statusBadge = document.getElementById("steam-status-badge");
  const statusText = document.getElementById("steam-status-text");
  const cardProgress = document.getElementById("steam-card-progress");
  const cardPercent = document.getElementById("steam-card-percent");
  const cardBytes = document.getElementById("steam-card-bytes");
  const cardSpeed = document.getElementById("steam-card-speed");
  const cardEta = document.getElementById("steam-card-eta");

  const topTitle = document.getElementById("top-dl-title");
  const topBar = document.getElementById("top-dl-bar");
  const topPercent = document.getElementById("top-dl-percent");
  const topSub = document.getElementById("top-dl-sub");

  if (emptyQueue) emptyQueue.hidden = true;
  if (activeCard) activeCard.hidden = false;
  if (queueCount) queueCount.textContent = "Up Next (1)";

  if (cardTitle) {
    cardTitle.textContent = cleanName;
    cardTitle.title = cleanName;
  }
  if (cardPath) {
    cardPath.textContent = destDir;
    cardPath.title = destDir;
  }
  if (statusBadge) {
    statusBadge.className = "steam-card-status-pill";
    if (statusText) statusText.textContent = "Fetching download link…";
  }
  if (cardProgress) {
    cardProgress.style.width = "0%";
    cardProgress.className = "steam-progress-fill";
  }
  if (cardPercent) cardPercent.textContent = "0%";
  if (cardBytes) {
    cardBytes.textContent = "0.00 GB / Calculando…";
    cardBytes.title = "Calculando tamanho total…";
  }
  if (cardSpeed) cardSpeed.textContent = "Conectando…";
  if (cardEta) {
    cardEta.textContent = "Obtendo autorização segura…";
    cardEta.title = "Obtendo autorização segura da Epic Games…";
  }

  if (topTitle) topTitle.textContent = `Baixando ${cleanName}…`;
  if (topBar) topBar.style.width = "0%";
  if (topPercent) topPercent.textContent = "0%";
  if (topSub) topSub.textContent = "Fetching download link…";

  let finalBlob = { ...blob };

  // Se o blob ainda não possui a URL pré-assinada da AWS S3, resolve silenciosamente em background
  if (!finalBlob.url) {
    try {
      const captured = await invoke("epic_open_download_window", { targetVersion: blob.version });
      if (captured && captured.url) {
        const cleanZipName = (captured.name && captured.name.endsWith(".zip") && captured.name.startsWith("Linux_Unreal_Engine"))
          ? captured.name
          : blob.name;
        finalBlob = { ...blob, ...captured, name: cleanZipName };
        if (statusText) statusText.textContent = "Downloading…";
        if (cardEta) {
          cardEta.textContent = "Iniciando download AWS S3…";
          cardEta.title = "Iniciando transferência da AWS S3…";
        }
        if (topSub) topSub.textContent = "Iniciando download…";
      } else {
        throw new Error("A Epic Games não retornou a URL direta para esta versão.");
      }
    } catch (err) {
      showToast(`Erro ao preparar download: ${err}`, true);
      isDownloadActive = false;
      if (emptyQueue) emptyQueue.hidden = false;
      if (activeCard) activeCard.hidden = true;
      if (queueCount) queueCount.textContent = "Up Next (0)";
      updateTopDownloadWidgetVisibility();
      return;
    }
  }

  let downloadDone = false;
  const unlistenExtract = await listen("engine-extract-progress", (event) => {
    const pct = Math.round(event.payload.percent);
    steamNetSpeed = 0.0;
    steamDiskSpeed = 75.0; // Velocidade de gravação no disco durante a extração
    updateSteamMetricsDisplay();

    if (downloadDone) {
      if (statusBadge) {
        statusBadge.className = "steam-card-status-pill extracting";
        if (statusText) statusText.textContent = "Extracting…";
      }
      if (cardProgress) {
        cardProgress.style.width = `${event.payload.percent}%`;
        cardProgress.className = "steam-progress-fill extracting";
      }
      if (cardPercent) cardPercent.textContent = `${pct}%`;
      if (cardBytes) {
        cardBytes.textContent = "Descompactando arquivos…";
        cardBytes.title = `Descompactando arquivos em ${destDir}`;
      }
      if (cardSpeed) cardSpeed.textContent = "Gravando no disco";
      if (cardEta) {
        const raw = event.payload.currentFile || "Instalando…";
        const clean = raw.replace(/^(inflating|extracting|creating):\s*/i, "").trim();
        cardEta.textContent = truncateFilePath(raw, 50);
        cardEta.title = clean;
      }

      if (topTitle) topTitle.textContent = `Extraindo ${cleanName}…`;
      if (topBar) topBar.style.width = `${event.payload.percent}%`;
      if (topPercent) topPercent.textContent = `${pct}%`;
      if (topSub) topSub.textContent = "Instalando arquivos no disco…";
    }
  });

  try {
    const downloadPromise = invoke("epic_download_and_install", { blob: finalBlob, destDir });
    downloadDone = true;
    const install = await downloadPromise;

    steamNetSpeed = 0.0;
    steamDiskSpeed = 0.0;
    updateSteamMetricsDisplay();

    if (statusBadge) {
      statusBadge.className = "steam-card-status-pill completed";
      if (statusText) statusText.textContent = "Completed";
    }
    if (cardProgress) cardProgress.style.width = "100%";
    if (cardPercent) cardPercent.textContent = "100%";
    if (cardSpeed) cardSpeed.textContent = "Instalada";
    if (cardEta) cardEta.textContent = "Pronta para uso!";

    showToast(`Engine ${install.version} instalada e registrada com sucesso!`);
    await refreshEngines();
    await refreshProjects();

    setTimeout(() => {
      if (activeCard) activeCard.hidden = true;
      if (emptyQueue) emptyQueue.hidden = false;
      if (queueCount) queueCount.textContent = "Up Next (0)";
    }, 6000);
  } catch (err) {
    if (!String(err).includes("cancelado")) {
      showToast(`Erro durante download/instalação: ${err}`, true);
    }
    steamNetSpeed = 0.0;
    steamDiskSpeed = 0.0;
    updateSteamMetricsDisplay();
    if (activeCard) activeCard.hidden = true;
    if (emptyQueue) emptyQueue.hidden = false;
    if (queueCount) queueCount.textContent = "Up Next (0)";
  } finally {
    unlistenExtract();
    isDownloadActive = false;
    isDownloadPaused = false;
    currentDownloadDestDir = "";
    updateTopDownloadWidgetVisibility();
  }
}

listen("engine-download-progress", (event) => {
  const { percent, bytesDownloaded, totalBytes, speedMbps, isPaused } = event.payload;
  const pct = Math.round(percent);
  const dlGb = (bytesDownloaded / 1e9).toFixed(2);
  const totalGb = (totalBytes / 1e9).toFixed(2);

  const statusBadge = document.getElementById("steam-status-badge");
  const statusText = document.getElementById("steam-status-text");
  const pauseIcon = document.getElementById("btn-steam-pause-icon");
  const pauseText = document.getElementById("btn-steam-pause-text");

  if (isPaused) {
    isDownloadPaused = true;
    steamNetSpeed = 0.0;
    steamDiskSpeed = 0.0;
    updateSteamMetricsDisplay();
    if (pauseIcon) pauseIcon.textContent = "▶";
    if (pauseText) pauseText.textContent = "Retomar";
    if (statusBadge) {
      statusBadge.className = "steam-card-status-pill paused";
      if (statusText) statusText.textContent = "Paused";
    }
  } else {
    isDownloadPaused = false;
    steamNetSpeed = speedMbps;
    steamDiskSpeed = speedMbps;
    if (speedMbps > steamPeakSpeed) {
      steamPeakSpeed = speedMbps;
    }
    updateSteamMetricsDisplay();
    if (pauseIcon) pauseIcon.textContent = "⏸";
    if (pauseText) pauseText.textContent = "Pausar";
    if (statusBadge && statusBadge.classList.contains("paused")) {
      statusBadge.className = "steam-card-status-pill";
      if (statusText) statusText.textContent = "Downloading…";
    }
  }

  // Atualiza o Card Ativo na aba de Downloads
  const cardProgress = document.getElementById("steam-card-progress");
  const cardPercent = document.getElementById("steam-card-percent");
  const cardBytes = document.getElementById("steam-card-bytes");
  const cardSpeed = document.getElementById("steam-card-speed");
  const cardEta = document.getElementById("steam-card-eta");

  if (cardProgress) cardProgress.style.width = `${percent}%`;
  if (cardPercent) cardPercent.textContent = `${pct}%`;
  if (cardBytes) {
    cardBytes.textContent = `${dlGb} GB / ${totalGb} GB`;
    cardBytes.title = `${dlGb} GB baixados de ${totalGb} GB`;
  }
  if (cardSpeed) cardSpeed.textContent = isPaused ? "Pausado" : `${speedMbps.toFixed(1)} MB/s`;

  if (cardEta) {
    let etaText = "Calculando tempo restante…";
    if (isPaused) {
      etaText = "Download pausado";
    } else {
      const remainingBytes = Math.max(0, totalBytes - bytesDownloaded);
      if (speedMbps > 0.1 && remainingBytes > 0) {
        const remainingSec = remainingBytes / (speedMbps * 1e6);
        if (remainingSec < 60) {
          etaText = `~${Math.round(remainingSec)}s restantes`;
        } else {
          const mins = Math.round(remainingSec / 60);
          etaText = `~${mins} min restantes`;
        }
      }
    }
    cardEta.textContent = etaText;
    cardEta.title = etaText;
  }

  // Atualização em tempo real do widget na Topbar (SOMENTE se NÃO estiver na aba Downloads)
  if (currentView !== "downloads") {
    const topActive = document.getElementById("top-download-active");
    const topBtn = document.getElementById("btn-open-epic-downloader");
    const topBar = document.getElementById("top-dl-bar");
    const topPercent = document.getElementById("top-dl-percent");
    const topSub = document.getElementById("top-dl-sub");

    if (topActive && topBar && topPercent && topSub) {
      if (topBtn) topBtn.hidden = false;
      topActive.hidden = false;
      topBar.style.width = `${percent}%`;
      topPercent.textContent = `${pct}%`;
      topSub.textContent = isPaused ? `Pausado • ${dlGb}/${totalGb} GB` : `${speedMbps.toFixed(1)} MB/s • ${dlGb}/${totalGb} GB`;
    }
  } else {
    // Na aba downloads, garante que fica estritamente oculta
    const topActive = document.getElementById("top-download-active");
    if (topActive) topActive.hidden = true;
  }
});

// Clicar no widget de download na barra superior leva direto para a aba Downloads
const topDownloadActiveWidget = document.getElementById("top-download-active");
if (topDownloadActiveWidget) {
  topDownloadActiveWidget.style.cursor = "pointer";
  topDownloadActiveWidget.addEventListener("click", () => {
    switchView("downloads");
  });
}

// ==========================================================================
// Biblioteca / Vault Unreal (Plugins, Projetos e Assets)
// ==========================================================================

let vaultItems = [];
let vaultFilteredItems = [];
let vaultLoaded = false;
let vaultFilter = "all";
let vaultSearchQuery = "";
let vaultRenderLimit = 40;
let selectedVaultItem = null;
let isVaultDownloading = false;

let isVaultRefreshing = false;
let vaultAccountId = null;
let vaultGeneration = 0;

function setVaultAccount(accountId, forceReset = false) {
  accountId = accountId || null;
  if (!forceReset && accountId === vaultAccountId) return;
  vaultAccountId = accountId;
  vaultGeneration++;
  vaultItems = [];
  vaultFilteredItems = [];
  vaultLoaded = false;
  isVaultRefreshing = false;
  selectedVaultItem = null;
  const modal = document.getElementById("vault-action-modal");
  if (modal) modal.hidden = true;
  const loading = document.getElementById("vault-loading");
  if (loading) loading.hidden = true;
  renderVaultGrid();
}

async function refreshVault(forceRefresh = false) {
  const status = await refreshEpicStatus();
  if (!status?.logged_in || !vaultAccountId) {
    renderVaultGrid();
    return;
  }
  if (isVaultRefreshing) return;
  isVaultRefreshing = true;
  const generation = vaultGeneration;

  const loading = document.getElementById("vault-loading");
  const empty = document.getElementById("vault-empty");
  const refreshBtn = document.getElementById("btn-refresh-vault");

  if (!vaultLoaded && loading) loading.hidden = false;
  if (empty) empty.hidden = true;
  if (refreshBtn && forceRefresh) {
    refreshBtn.disabled = true;
    refreshBtn.innerHTML = `${uiIcon("refresh")} Sincronizando…`;
  }

  try {
    const items = await invoke("list_vault_items", { forceRefresh });
    if (generation !== vaultGeneration) return;
    vaultItems = items || [];
    vaultLoaded = true;
    if (loading) loading.hidden = true;
    applyVaultFilters();
    if (forceRefresh) {
      showToast(`Biblioteca sincronizada (${vaultItems.length} itens encontrados)`);
    }
  } catch (err) {
    if (generation !== vaultGeneration) return;
    vaultItems = [];
    vaultFilteredItems = [];
    vaultLoaded = false;
    renderVaultGrid();
    if (loading) loading.hidden = true;
    showToast(`Erro ao carregar biblioteca da Epic: ${err}`, true);
  } finally {
    if (generation !== vaultGeneration) return;
    isVaultRefreshing = false;
    if (refreshBtn) {
      refreshBtn.disabled = false;
      refreshBtn.innerHTML = `${uiIcon("refresh")} Sincronizar`;
    }
  }
}

function applyVaultFilters() {
  const query = vaultSearchQuery.toLowerCase().trim();
  vaultFilteredItems = vaultItems.filter((item) => {
    if (vaultFilter !== "all" && item.item_type !== vaultFilter) {
      return false;
    }
    if (query) {
      const matchTitle = item.title.toLowerCase().includes(query);
      const matchDev = item.developer.toLowerCase().includes(query);
      const matchDesc = item.description.toLowerCase().includes(query);
      const matchVer = item.releases.some((r) =>
        r.compatible_apps.some((app) => app.toLowerCase().includes(query))
      );
      if (!matchTitle && !matchDev && !matchDesc && !matchVer) {
        return false;
      }
    }
    return true;
  });

  renderVaultGrid(true);
}

function renderVaultGrid(resetLimit = true) {
  const grid = document.getElementById("vault-grid");
  const empty = document.getElementById("vault-empty");
  if (!grid) return;

  if (resetLimit) vaultRenderLimit = 40;
  grid.innerHTML = "";

  const emptyTitle = document.getElementById("vault-empty-title");
  const emptyHint = document.getElementById("vault-empty-hint");
  const refreshBtn = document.getElementById("btn-refresh-vault");
  if (refreshBtn) refreshBtn.disabled = !vaultAccountId;
  if (emptyTitle) emptyTitle.textContent = vaultAccountId ? "Nenhum item encontrado." : "Conecte sua conta Epic.";
  if (emptyHint) emptyHint.textContent = vaultAccountId
    ? "Verifique o termo pesquisado ou sincronize a biblioteca com sua conta Epic."
    : "Faça login para visualizar seus plugins, projetos e assets.";
  if (!vaultAccountId) {
    if (empty) empty.hidden = false;
    return;
  }

  if (vaultFilteredItems.length === 0) {
    if (empty) empty.hidden = false;
    return;
  }
  if (empty) empty.hidden = true;

  const slice = vaultFilteredItems.slice(0, vaultRenderLimit);
  for (const item of slice) {
    grid.appendChild(createVaultCard(item));
  }

  if (vaultFilteredItems.length > vaultRenderLimit) {
    const loadMoreBox = document.createElement("div");
    loadMoreBox.style.gridColumn = "1 / -1";
    loadMoreBox.style.textAlign = "center";
    loadMoreBox.style.padding = "24px 0";
    loadMoreBox.innerHTML = `
      <button class="btn btn-ghost" id="btn-vault-load-more" style="padding: 8px 24px;">
        Carregar mais (${vaultFilteredItems.length - vaultRenderLimit} itens restantes)…
      </button>
    `;
    loadMoreBox.querySelector("#btn-vault-load-more").addEventListener("click", () => {
      vaultRenderLimit += 40;
      renderVaultGrid(false);
    });
    grid.appendChild(loadMoreBox);
  }
}

function createVaultCard(item) {
  const card = document.createElement("div");
  card.className = "vault-card";

  const typeLabels = {
    plugin: "Plugin",
    project: "Projeto",
    asset_pack: "Conteúdo",
    other: "Item",
  };
  const typeLabel = typeLabels[item.item_type] || "Item";

  const compatibleVersions = Array.from(
    new Set(
      item.releases.flatMap((r) =>
        r.compatible_apps.map((app) => app.replace("UE_", ""))
      )
    )
  ).slice(0, 4);

  const isSafeThumb = item.thumbnail_url && /^https?:\/\//i.test(item.thumbnail_url);
  const thumbHtml = isSafeThumb
    ? `<img src="${escapeHtml(item.thumbnail_url)}" alt="${escapeHtml(item.title)}" loading="lazy" />`
    : `<div class="vault-card-placeholder">${uiIcon("box")}</div>`;

  card.innerHTML = `
    <div class="vault-card-cover">
      ${thumbHtml}
      <span class="vault-type-badge ${escapeHtml(item.item_type)}">${escapeHtml(typeLabel)}</span>
    </div>
    <div class="vault-card-body">
      <h3 class="vault-card-title" title="${escapeHtml(item.title)}">${escapeHtml(item.title)}</h3>
      <span class="vault-card-dev" title="${escapeHtml(item.developer)}">${escapeHtml(item.developer)}</span>
      <div class="vault-card-versions">
        ${compatibleVersions.map((v) => `<span class="vault-ver-tag">UE ${escapeHtml(v)}</span>`).join("")}
      </div>
      <div class="vault-card-footer">
        <button class="btn-vault-install">
          <span>Opções de Instalação ›</span>
        </button>
      </div>
    </div>
  `;

  card.querySelector(".btn-vault-install").addEventListener("click", () => {
    openVaultActionModal(item);
  });

  return card;
}

// ---------- Modal de Ação do Vault ----------
function openVaultActionModal(item) {
  selectedVaultItem = item;
  const modal = document.getElementById("vault-action-modal");
  const title = document.getElementById("vault-modal-title");
  const dev = document.getElementById("vault-modal-dev");
  const desc = document.getElementById("vault-modal-desc");
  const thumb = document.getElementById("vault-modal-thumb");
  const releaseSelect = document.getElementById("select-vault-release");
  const projectSelect = document.getElementById("select-vault-target-project");
  const engineSelect = document.getElementById("select-vault-target-engine");
  const newProjEngineSelect = document.getElementById("select-vault-new-proj-engine");
  const newProjDirInput = document.getElementById("input-vault-new-proj-dir");
  const newProjNameInput = document.getElementById("input-vault-new-proj-name");
  const progressBox = document.getElementById("vault-modal-progress-box");
  const confirmBtn = document.getElementById("btn-confirm-vault-action");

  if (!modal) return;

  if (title) title.textContent = item.title;
  if (dev) dev.textContent = `Por ${item.developer}`;
  if (desc) desc.textContent = item.description || "Sem descrição disponível.";
  if (thumb) {
    const isSafeThumb = item.thumbnail_url && /^https?:\/\//i.test(item.thumbnail_url);
    thumb.src = isSafeThumb ? item.thumbnail_url : "";
    thumb.style.display = isSafeThumb ? "block" : "none";
  }

  // Preencher versões disponíveis
  if (releaseSelect) {
    if (item.releases.length === 0) {
      releaseSelect.innerHTML = `<option value="">Nenhuma versão compatível listada</option>`;
    } else {
      releaseSelect.innerHTML = item.releases
        .map((r) => {
          const apps = r.compatible_apps.length > 0 ? ` [${r.compatible_apps.join(", ")}]` : "";
          const title = r.version_title || r.app_id;
          return `<option value="${escapeHtml(r.app_id)}">${escapeHtml(title)}${escapeHtml(apps)}</option>`;
        })
        .join("");
    }
  }

  // Preencher projetos monitorados
  if (projectSelect) {
    if (projects.length === 0) {
      projectSelect.innerHTML = `<option value="">Nenhum projeto monitorado encontrado</option>`;
    } else {
      projectSelect.innerHTML = projects
        .map((p) => `<option value="${escapeHtml(p.uproject_path)}">${escapeHtml(p.name)} (${escapeHtml(p.project_dir)})</option>`)
        .join("");
    }
  }

  // Preencher engines
  const engineOpts = engines.map((e) => `<option value="${escapeHtml(e.id)}">Unreal Engine ${escapeHtml(e.version)} (${escapeHtml(e.path)})</option>`).join("");
  if (engineSelect) engineSelect.innerHTML = engineOpts;
  if (newProjEngineSelect) newProjEngineSelect.innerHTML = engineOpts;


  // Sugerir nome e pasta para novo projeto
  if (newProjNameInput) {
    const cleanName = item.title.replace(/[^a-zA-Z0-9_]/g, "");
    newProjNameInput.value = cleanName || "NewProject";
  }
  if (newProjDirInput) {
    newProjDirInput.value = projectDirs.length > 0 ? projectDirs[0] : "";
  }

  // Selecionar ação padrão
  if (item.item_type === "project") {
    setVaultActionSelection("new_project");
  } else {
    setVaultActionSelection("project");
  }

  if (progressBox) progressBox.hidden = true;
  if (confirmBtn) {
    confirmBtn.disabled = false;
    confirmBtn.innerHTML = `<span>Baixar e Instalar</span>`;
  }

  modal.hidden = false;
}

function closeVaultActionModal() {
  const modal = document.getElementById("vault-action-modal");
  if (modal) modal.hidden = true;
  selectedVaultItem = null;
}

function setVaultActionSelection(action) {
  const cardProj = document.getElementById("card-vault-act-project");
  const cardEngine = document.getElementById("card-vault-act-engine");
  const cardNewProj = document.getElementById("card-vault-act-new-proj");

  const panelProj = document.getElementById("vault-panel-project");
  const panelEngine = document.getElementById("vault-panel-engine");
  const panelNewProj = document.getElementById("vault-panel-new-proj");

  cardProj?.classList.toggle("active", action === "project");
  cardEngine?.classList.toggle("active", action === "engine");
  cardNewProj?.classList.toggle("active", action === "new_project");

  const radio = document.querySelector(`input[name="vault-act"][value="${action}"]`);
  if (radio) radio.checked = true;

  if (panelProj) panelProj.hidden = action !== "project";
  if (panelEngine) panelEngine.hidden = action !== "engine";
  if (panelNewProj) panelNewProj.hidden = action !== "new_project";
}

// Listeners de alternância de ação no modal
document.querySelectorAll("input[name='vault-act']").forEach((radio) => {
  radio.addEventListener("change", (e) => {
    setVaultActionSelection(e.target.value);
  });
});
document.getElementById("card-vault-act-project")?.addEventListener("click", () => setVaultActionSelection("project"));
document.getElementById("card-vault-act-engine")?.addEventListener("click", () => setVaultActionSelection("engine"));
document.getElementById("card-vault-act-new-proj")?.addEventListener("click", () => setVaultActionSelection("new_project"));

// Botão Procurar pasta no modal do Vault
document.getElementById("btn-browse-vault-proj-dir")?.addEventListener("click", async () => {
  try {
    const selected = await openDialog({ directory: true, multiple: false });
    if (selected) {
      document.getElementById("input-vault-new-proj-dir").value = selected;
    }
  } catch (err) {
    console.error(err);
  }
});

// Botão Cancelar modal do Vault
document.getElementById("btn-cancel-vault-modal")?.addEventListener("click", () => {
  if (isVaultDownloading) {
    invoke("cancel_vault_download");
  }
  closeVaultActionModal();
});

// Botão Confirmar Ação (Baixar e Instalar)
document.getElementById("btn-confirm-vault-action")?.addEventListener("click", async () => {
  if (!selectedVaultItem) return;

  const releaseSelect = document.getElementById("select-vault-release");
  const appId = releaseSelect?.value;
  if (!appId) {
    showToast("Selecione uma versão compatível.", true);
    return;
  }

  const action = document.querySelector("input[name='vault-act']:checked")?.value || "project";
  const progressBox = document.getElementById("vault-modal-progress-box");
  const confirmBtn = document.getElementById("btn-confirm-vault-action");

  if (action === "project") {
    const uprojectPath = document.getElementById("select-vault-target-project")?.value;
    if (!uprojectPath) {
      showToast("Selecione um projeto de destino.", true);
      return;
    }

    try {
      isVaultDownloading = true;
      confirmBtn.disabled = true;
      confirmBtn.innerHTML = `<span>Instalando… ⏳</span>`;
      if (progressBox) progressBox.hidden = false;

      const installedPath = await invoke("install_vault_to_project", {
        catalogItemId: selectedVaultItem.id,
        appId,
        uprojectPath,
      });

      showToast(`Plugin instalado com sucesso em: ${installedPath}`);
      closeVaultActionModal();
      await refreshProjects();
    } catch (err) {
      showToast(`Falha na instalação: ${err}`, true);
    } finally {
      isVaultDownloading = false;
      confirmBtn.disabled = false;
      confirmBtn.innerHTML = `<span>Baixar e Instalar</span>`;
    }
  } else if (action === "engine") {
    const engineId = document.getElementById("select-vault-target-engine")?.value;
    if (!engineId) {
      showToast("Selecione uma engine instalada.", true);
      return;
    }

    try {
      isVaultDownloading = true;
      confirmBtn.disabled = true;
      confirmBtn.innerHTML = `<span>Instalando na Engine… ⏳</span>`;
      if (progressBox) progressBox.hidden = false;

      const installedPath = await invoke("install_vault_to_engine", {
        catalogItemId: selectedVaultItem.id,
        appId,
        engineId,
      });

      showToast(`Plugin instalado na Engine em: ${installedPath}`);
      closeVaultActionModal();
    } catch (err) {
      showToast(`Falha na instalação: ${err}`, true);
    } finally {
      isVaultDownloading = false;
      confirmBtn.disabled = false;
      confirmBtn.innerHTML = `<span>Baixar e Instalar</span>`;
    }
  } else if (action === "new_project") {
    const name = document.getElementById("input-vault-new-proj-name")?.value.trim();
    const parentDir = document.getElementById("input-vault-new-proj-dir")?.value.trim();
    const engineId = document.getElementById("select-vault-new-proj-engine")?.value;

    if (!name || !/^[a-zA-Z][a-zA-Z0-9_]*$/.test(name)) {
      showToast("Nome do projeto inválido (apenas letras, números e _).", true);
      return;
    }
    if (!parentDir) {
      showToast("Selecione a pasta de destino.", true);
      return;
    }
    if (!engineId) {
      showToast("Selecione uma engine associada.", true);
      return;
    }

    try {
      isVaultDownloading = true;
      confirmBtn.disabled = true;
      confirmBtn.innerHTML = `<span>Criando Projeto… ⏳</span>`;
      if (progressBox) progressBox.hidden = false;

      const newProj = await invoke("create_project_from_vault", {
        catalogItemId: selectedVaultItem.id,
        appId,
        projectName: name,
        parentDir,
        engineId,
      });

      if (!projectDirs.includes(parentDir)) {
        await invoke("add_project_dir", { path: parentDir });
        await refreshProjectDirs();
      }

      showToast(`Projeto ${newProj.name} criado com sucesso a partir do Vault!`);
      closeVaultActionModal();
      await refreshProjects();
    } catch (err) {
      showToast(`Falha ao criar projeto: ${err}`, true);
    } finally {
      isVaultDownloading = false;
      confirmBtn.disabled = false;
      confirmBtn.innerHTML = `<span>Baixar e Instalar</span>`;
    }
  }
});

// Listener de Progresso de Download do Vault
listen("vault-download-progress", (event) => {
  const p = event.payload;
  const statusEl = document.getElementById("vault-progress-status");
  const barEl = document.getElementById("vault-progress-bar");
  const pctEl = document.getElementById("vault-progress-percent");
  const bytesEl = document.getElementById("vault-progress-bytes");
  const speedEl = document.getElementById("vault-progress-speed");

  if (statusEl) statusEl.textContent = p.title;
  if (barEl) barEl.style.width = `${Math.min(100, Math.max(0, p.percentage))}%`;
  if (pctEl) pctEl.textContent = `${Math.round(p.percentage)}%`;
  if (bytesEl && p.total_bytes > 0) {
    const dlMb = (p.downloaded_bytes / (1024 * 1024)).toFixed(1);
    const totMb = (p.total_bytes / (1024 * 1024)).toFixed(1);
    bytesEl.textContent = `${dlMb} MB / ${totMb} MB`;
  }
  if (speedEl && p.speed_bytes_per_sec > 0) {
    const spdMb = (p.speed_bytes_per_sec / (1024 * 1024)).toFixed(1);
    speedEl.textContent = `${spdMb} MB/s`;
  }
});

// Filtros do Vault (Tabs)
document.querySelectorAll("#vault-filter-tabs .filter-tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll("#vault-filter-tabs .filter-tab").forEach((t) => t.classList.remove("active"));
    tab.classList.add("active");
    vaultFilter = tab.dataset.filter || "all";
    applyVaultFilters();
  });
});

// Busca com debounce
let vaultSearchTimer = null;
document.getElementById("vault-search-input")?.addEventListener("input", (e) => {
  clearTimeout(vaultSearchTimer);
  vaultSearchTimer = setTimeout(() => {
    vaultSearchQuery = e.target.value;
    applyVaultFilters();
  }, 200);
});

// Botão de sincronização manual da biblioteca
document.getElementById("btn-refresh-vault")?.addEventListener("click", async () => {
  await refreshVault(true);
});

// ---------- LIVE UPDATE SYSTEM ----------
let availableUpdate = null;
let isAppUpdating = false;

function formatBytes(bytes) {
  if (!bytes || bytes <= 0) return "0 MB";
  return (bytes / (1024 * 1024)).toFixed(1) + " MB";
}

function formatDate(isoStr) {
  if (!isoStr) return "";
  try {
    const d = new Date(isoStr);
    return d.toLocaleDateString("pt-BR", { day: "2-digit", month: "2-digit", year: "numeric" });
  } catch {
    return isoStr;
  }
}

async function checkForAppUpdates(manual = false) {
  const pillBtn = document.getElementById("btn-update-available");
  const pillText = document.getElementById("btn-update-pill-text");

  try {
    if (manual) {
      showToast("Verificando atualizações no repositório...", "info");
    }

    const info = await invoke("check_app_update");
    if (info && info.has_update) {
      availableUpdate = info;
      if (pillText) pillText.textContent = `Nova versão v${info.latest_version} disponível!`;
      if (pillBtn) pillBtn.hidden = false;

      if (manual) {
        openAppUpdateModal();
      } else {
        showToast(`Nova versão v${info.latest_version} do ArcForge disponível!`, "info");
      }
    } else {
      if (pillBtn) pillBtn.hidden = true;
      if (manual) {
        const curVer = info?.current_version || "0.1.0";
        showToast(`Você já está utilizando a versão mais recente (v${curVer})!`, "success");
      }
    }
  } catch (err) {
    console.error("Erro ao verificar atualizações:", err);
    if (manual) {
      showToast("Não foi possível verificar atualizações: " + err, "error");
    }
  }
}

function openAppUpdateModal() {
  if (!availableUpdate) return;

  const modal = document.getElementById("modal-app-update");
  const curVerEl = document.getElementById("update-current-version");
  const latVerEl = document.getElementById("update-latest-version");
  const dateEl = document.getElementById("update-release-date");
  const sizeEl = document.getElementById("update-release-size");
  const notesEl = document.getElementById("update-release-notes");
  const progressBox = document.getElementById("update-progress-container");
  const startBtn = document.getElementById("btn-start-live-update");

  if (curVerEl) curVerEl.textContent = `v${availableUpdate.current_version}`;
  if (latVerEl) latVerEl.textContent = `v${availableUpdate.latest_version}`;
  if (dateEl) dateEl.textContent = `Lançamento: ${formatDate(availableUpdate.published_at)}`;
  if (sizeEl) sizeEl.textContent = `Tamanho: ~${formatBytes(availableUpdate.asset_size)}`;

  if (notesEl) {
    notesEl.textContent = availableUpdate.release_notes || "Sem notas de lançamento detalhadas.";
  }

  if (progressBox) progressBox.hidden = true;
  if (startBtn) {
    startBtn.disabled = false;
    startBtn.innerHTML = availableUpdate.automatic_update_ready ? `<span>Atualizar Agora</span>` : `<span>Ver release oficial</span>`;
  }

  if (modal) modal.hidden = false;
}

function closeAppUpdateModal() {
  if (isAppUpdating) {
    showToast("Atualização em andamento. Por favor aguarde.", "warning");
    return;
  }
  const modal = document.getElementById("modal-app-update");
  if (modal) modal.hidden = true;
}

async function startLiveUpdate() {
  if (!availableUpdate) return;
  if (!availableUpdate.automatic_update_ready) {
    await openUrl("https://github.com/ricardofuly/ArcForge/releases/latest");
    return;
  }
  if (!availableUpdate.asset_url) {
    showToast("Nenhum binário direto encontrado para esta versão. Abrindo GitHub...", "info");
    if (availableUpdate.html_url) {
      openUrl(availableUpdate.html_url);
    }
    return;
  }

  isAppUpdating = true;
  const progressBox = document.getElementById("update-progress-container");
  const startBtn = document.getElementById("btn-start-live-update");
  const laterBtn = document.getElementById("btn-remind-later-update");
  const closeBtn = document.getElementById("btn-close-update-modal");

  if (progressBox) progressBox.hidden = false;
  if (startBtn) {
    startBtn.disabled = true;
    startBtn.innerHTML = `<span>Atualizando…</span>`;
  }
  if (laterBtn) laterBtn.disabled = true;
  if (closeBtn) closeBtn.disabled = true;

  try {
    await invoke("download_and_apply_update", {
      assetUrl: availableUpdate.asset_url,
      assetName: availableUpdate.asset_name || "update_package",
    });
  } catch (err) {
    console.error("Erro ao aplicar atualização:", err);
    isAppUpdating = false;
    if (startBtn) {
      startBtn.disabled = false;
      startBtn.innerHTML = `<span>Tentar Novamente</span>`;
    }
    if (laterBtn) laterBtn.disabled = false;
    if (closeBtn) closeBtn.disabled = false;
    const statusEl = document.getElementById("update-progress-status");
    if (statusEl) statusEl.textContent = "Erro ao atualizar: " + err;
    showToast("Erro ao instalar atualização: " + err, "error");
  }
}

// Escuta o progresso do live update
listen("app_update_progress", (event) => {
  const p = event.payload;
  const statusEl = document.getElementById("update-progress-status");
  const barEl = document.getElementById("update-progress-bar");
  const pctEl = document.getElementById("update-progress-percent");
  const bytesEl = document.getElementById("update-progress-bytes");
  const speedEl = document.getElementById("update-progress-speed");

  if (statusEl) {
    if (p.status === "installing") {
      statusEl.textContent = "Instalando atualização e reiniciando…";
    } else {
      statusEl.textContent = p.message || "Baixando atualização…";
    }
  }

  if (barEl) barEl.style.width = `${Math.min(100, Math.max(0, p.percentage))}%`;
  if (pctEl) pctEl.textContent = `${Math.round(p.percentage)}%`;

  if (bytesEl && p.total_bytes > 0) {
    const dlMb = (p.downloaded_bytes / (1024 * 1024)).toFixed(1);
    const totMb = (p.total_bytes / (1024 * 1024)).toFixed(1);
    bytesEl.textContent = `${dlMb} MB / ${totMb} MB`;
  }

  if (speedEl && p.speed_mbps > 0) {
    speedEl.textContent = `${p.speed_mbps.toFixed(1)} MB/s`;
  }
});

// Event listeners do sistema de update
document.getElementById("btn-update-available")?.addEventListener("click", openAppUpdateModal);
document.getElementById("btn-check-updates")?.addEventListener("click", () => checkForAppUpdates(true));
document.getElementById("btn-notifications")?.addEventListener("click", () => checkForAppUpdates(true));
document.getElementById("btn-close-update-modal")?.addEventListener("click", closeAppUpdateModal);
document.getElementById("btn-remind-later-update")?.addEventListener("click", closeAppUpdateModal);
document.getElementById("btn-start-live-update")?.addEventListener("click", startLiveUpdate);
document.getElementById("btn-view-github-release")?.addEventListener("click", () => {
  if (availableUpdate?.html_url) {
    openUrl(availableUpdate.html_url);
  }
});

// ---------- Boot ----------
(async function init() {
  buildUiReady = initBuildUi();
  await buildUiReady;
  if (isWindows) {
    document.querySelectorAll(".modal-subtitle").forEach((el) => {
      if (el.textContent.includes("Compilações oficiais da Epic Games para Linux")) el.textContent = "Instale a engine pelo Epic Games Launcher e registre sua pasta aqui.";
    });
    const choice = document.querySelector("#choice-download-epic small");
    if (choice) choice.textContent = "Instalação oficial pelo Epic Games Launcher";
  }
  // Garante que o widget ativo começa estritamente oculto se não houver download em andamento
  const topActive = document.getElementById("top-download-active");
  const topBtn = document.getElementById("btn-open-epic-downloader");
  if (topActive) topActive.hidden = true;
  if (topBtn) topBtn.hidden = false;

  // Projects remain accessible while Epic authentication/network requests run.
  const accountAndGpu = Promise.allSettled([refreshEpicStatus(), checkGpuCompatibility()]);
  await Promise.all([refreshDetectedIdes(), refreshEngines(), refreshProjectDirs()]);
  await refreshProjects();
  await accountAndGpu;

  // Pré-carrega o cache do Vault em segundo plano para resposta instantânea ao abrir a aba
  refreshVault(false).catch((err) => console.warn("Pré-carregamento da biblioteca em background:", err));

  // Checagem silenciosa de novas versões do app
  setTimeout(() => checkForAppUpdates(false), 2000);
})();

