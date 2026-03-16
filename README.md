# Proyecto: LAN P2P File Deployer (BitTorrent)

Aplicación de escritorio construida con Rust y Tauri 2.0 diseñada para distribuir archivos pesados simultáneamente a otras computadoras en un laboratorio de red local, utilizando tecnología P2P para maximizar el ancho de banda y automatización en segundo plano. Tanto la computadora emisora como la receptora de los archivos funcionará de tal manera que mediante la aplicación pueda enviar y recibir archivos. Es decir, digamos que tenemos 50 computadoras en la red, de las cuales solo 20 computadoras tienen instalada la aplicación, entonces podemos ver las 50 computadoras conectadas, pero se especifica cuáles son las 20 a las que podemos enviar archivos, y cuales son las 30 no disponibles. Entonces, cualquiera de las 20 computadoras que contienen la aplicación, pueden elegir a cuáles computadoras se quiere enviar un archivo a elección, cual sea, y debe definir en qué lugar se va a guardar ese archivo en las computadoras destino. Y la aplicación se encarga también de aplicar la transmisión mediante bit torrent para optimizar el envío de este archivo a las demás computadoras.

## Características Principales
- **Distribución P2P:** Uso de BitTorrent a nivel local para evitar cuellos de botella en un solo servidor.
- **Rutas Forzadas:** Capacidad del emisor para dictar en qué carpeta se guardará el archivo en las máquinas de destino.
- **Ejecución Silenciosa:** Los clientes receptores operan en segundo plano sin requerir clics ni confirmaciones del usuario.

## Stack Tecnológico
- **Core:** Rust
- **Framework UI:** Tauri 2.0
- **Asincronía & Red:** Tokio
- **Frontend:** HTML/CSS/TypeScript (Vite)

## Roadmap (Paso a Paso)
- [ ] Escribir lógica Emisor y Receptor en background que hará posible la transmisión.
- [ ] Implementar fragmentación de archivos y creación de hashes.
- [ ] Integrar motor BitTorrent para la transferencia de piezas.
- [ ] Conectar Rust con la interfaz gráfica de Tauri.
- [ ] Pruebas de latencia y despliegue en Windows.