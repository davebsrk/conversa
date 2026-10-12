# Conversa

Cliente de escritorio ligero para WhatsApp Web, escrito en Rust con [Tauri](https://tauri.app).
Para Windows y Linux.

> **Aviso:** Conversa no está afiliado, asociado ni respaldado por WhatsApp ni por Meta Platforms.
> "WhatsApp" es una marca registrada de sus respectivos propietarios.

## Por qué

La app oficial de escritorio es lenta al arrancar, consume mucha RAM y sigue gastando recursos en
segundo plano. Conversa carga el mismo WhatsApp Web en la webview del sistema (WebView2 en Windows,
WebKitGTK en Linux), sin capas extra, y gestiona activamente la memoria cuando no la estás usando.

No reimplementa el protocolo de WhatsApp: es WhatsApp Web oficial dentro de una ventana ligera,
así que tu cuenta no corre riesgo de baneo por usar un cliente no oficial.

## Estado

En desarrollo temprano. Consulta el [plan](PLAN.md).

## Rendimiento

Medido con [`tools/measure.ps1`](tools/measure.ps1), que suma todo el árbol de procesos
(incluidos los procesos de WebView2). "Memoria" equivale a la columna del Administrador de tareas.

| Escenario (Windows 11) | App oficial 2.2639.100 | Conversa |
|---|---|---|
| Minimizada, en reposo — memoria | 399 MB | _pendiente_ |
| Minimizada, en reposo — commit | 1008 MB | _pendiente_ |
| Minimizada, en reposo — CPU | ~0 % | _pendiente_ |
| Tamaño del ejecutable | — | 4,2 MB |

## Compilar

Requisitos: Rust estable. En Linux, además:

```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev
```

```bash
cargo run --release
```

## Licencia

Con licencia dual, a tu elección:

- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT ([LICENSE-MIT](LICENSE-MIT))
