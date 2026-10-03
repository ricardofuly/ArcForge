// Tauri handles dragging and double-click maximize on data-tauri-drag-region.
const appWindow = window.__TAURI__?.window?.getCurrentWindow();

if (appWindow) {
  const maximizeButton = document.getElementById("window-maximize");
  let stateRevision = 0;

  async function syncMaximizedState() {
    const revision = ++stateRevision;
    const maximized = await appWindow.isMaximized();
    if (revision !== stateRevision) return;
    const label = maximized ? "Restaurar" : "Maximizar";
    maximizeButton.title = label;
    maximizeButton.setAttribute("aria-label", label);
    document.getElementById("window-maximize-icon").setAttribute("d", maximized
      ? "M5.5 5.5v-3h8v8h-3m-8-5h8v8h-8z"
      : "M3.5 3.5h9v9h-9z");
  }

  function bindAction(id, action) {
    document.getElementById(id).addEventListener("click", async () => {
      try { await action(); }
      catch (error) {
        console.error("Falha no controle da janela:", error);
        const toast = document.getElementById("toast");
        toast.textContent = "Não foi possível executar a ação da janela. Tente novamente.";
        toast.classList.add("error");
        toast.hidden = false;
      }
    });
  }

  bindAction("window-minimize", () => appWindow.minimize());
  bindAction("window-maximize", async () => {
    await appWindow.toggleMaximize();
    await syncMaximizedState();
  });
  bindAction("window-close", () => appWindow.close());
  appWindow.onResized(() => syncMaximizedState().catch(console.error)).catch(console.error);
  syncMaximizedState().catch(console.error);
}
