# Conversa

Cliente de escritorio ligero para WhatsApp Web, escrito en Rust con [Tauri](https://tauri.app).
Para Windows y Linux.

> **Aviso:** Conversa no está afiliado, asociado ni respaldado por WhatsApp ni por Meta Platforms.
> "WhatsApp" es una marca registrada de sus respectivos propietarios.

## Por qué

La app oficial de escritorio arranca lenta, consume mucha RAM y sigue gastando recursos en
segundo plano. Conversa carga el mismo WhatsApp Web en la webview del sistema (WebView2 en
Windows, WebKitGTK en Linux) y, en cuanto la minimizas o la mandas a la bandeja, le pide a la
webview que libere memoria sin cortar la conexión: los mensajes y las notificaciones siguen
llegando al momento.

No reimplementa el protocolo de WhatsApp: es WhatsApp Web oficial dentro de una ventana ligera,
así que no hay riesgo de baneo por usar un cliente no oficial.

## Funciones

- Todo lo que ofrece WhatsApp Web: chats, grupos, multimedia, notas de voz, estados, encuestas…
- Número de chats sin leer (sin contar los silenciados) en el icono de la barra de tareas y de la
  bandeja, como en la app oficial.
- Cerrar la ventana la manda a la bandeja; se sale desde el menú de la bandeja.
- Notificaciones nativas de Windows con la foto del contacto, **respuesta rápida** y botón
  **Marcar como leído** desde la propia notificación; al pulsarla se abre ese chat. En Linux, las notificaciones del sistema.
- **Ajustes** integrados en la ventana (engranaje en la barra lateral de WhatsApp o desde la
  bandeja): tamaño del texto (zoom), iniciar con el sistema, cerrar a la bandeja o salir,
  actualizaciones (avisar, instalar solas o no buscar), ocultar el texto de los mensajes y el atajo.
- Atajos globales configurables: mostrar u ocultar la ventana (por defecto <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>W</kbd>)
  y activar o desactivar el modo privacidad.
- Los enlaces se abren en tu navegador; las descargas van a tu carpeta Descargas.
- Los enlaces de WhatsApp (`wa.me`, `whatsapp://`, invitaciones a grupos) se abren en Conversa,
  tanto desde el navegador como desde un chat.
- Sin el aviso "Obtener WhatsApp para Windows".
- **Modo privacidad**: desenfoca la lista de chats y los mensajes hasta pasar el ratón por encima.
- **Aspecto**: ancho de la lista de chats y ocultar la barra de filtros, Estados, Canales o
  Comunidades.
- Recuerda el tamaño y la posición de la ventana.
- Avisa cuando hay una versión nueva y se actualiza con un clic (o sola, si lo eliges en Ajustes).

## Rendimiento

Medido con [`tools/measure.ps1`](tools/measure.ps1), que suma todo el árbol de procesos
(incluidos los procesos de WebView2). "Memoria" es la columna del Administrador de tareas.
Ambas apps minimizadas, en reposo, medidas a la vez durante 3 minutos en Windows 11:

| | App oficial 2.2639.100 | Conversa 0.1.0 |
|---|---|---|
| Memoria | 631 MB | **155 MB** |
| Commit (memoria reservada) | 1085 MB | **668 MB** |
| CPU | ~0 % | ~0 % |
| Tamaño del ejecutable | — | 6,5 MB |

En Linux, WebKitGTK no tiene una forma equivalente de liberar memoria al minimizar, así que allí
el ahorro es menor.

## Instalar

Descarga la última versión desde [Releases](https://github.com/davebsrk/conversa/releases/latest):

| Sistema | Archivo |
|---|---|
| Windows (instalador) | `Conversa_x.y.z_x64-setup.exe` |
| Windows (portable) | `Conversa_x.y.z_x64-portable.exe` |
| Debian / Ubuntu | `Conversa_x.y.z_amd64.deb` |
| Fedora / openSUSE | `Conversa-x.y.z-1.x86_64.rpm` |
| Cualquier Linux | `Conversa_x.y.z_amd64.AppImage` o `Conversa_x.y.z_x86_64.flatpak` |

Los instaladores de Windows no están firmados todavía: SmartScreen mostrará "Editor desconocido".
Pulsa *Más información → Ejecutar de todas formas*.

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
