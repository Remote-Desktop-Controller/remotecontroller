# Limpeza segura — 2026-09-30

Pedido: liberar caches/builds antigos sem apagar trabalho importante.

Removido pelos gerenciadores nativos:

- npm cache clean: cache de pacotes.
- npm cache npx rm: nove instalações temporárias npx identificadas por npm cache npx ls.
- pip cache purge: 110 arquivos / 120.7 MB; pacotes instalados não foram removidos.

Cache npm antes: 4.407.183.065 bytes, incluindo aproximadamente 1.98 GB de npx.
Total dessas limpezas: aproximadamente 4.5 GB. Espaço livre observado depois:
11.466.170.368 bytes (10.68 GiB). Medida varia com compilação/temporários ativos.

Antes do pedido adicional, cargo clean --profile dev já havia removido 4.7 GiB
de artifacts exclusivamente deste runtime, após erro de disco cheio. Toolchain
stable duplicada da instalação desta sessão também foi removida; 1.98.1 ficou
instalada e como default. Profiles dev/test sem debuginfo/incremental e jobs=2
reduzem nova ocupação.

Preservados: código, .git, documentos, banco/segredos, node_modules, bibliotecas
instaladas, modelos Hugging Face/Torch, runtimes do Codex, distribuições Gradle,
camera2-evidence, APK app-debug.apk (89.024.600 bytes), reports e test-results.
A pasta Android app/build de aproximadamente 23 GB não foi apagada: a maior
parte é evidência de câmera, não cache descartável.

Exclusão direta de `.next` e de transforms Gradle foi rejeitada pela revisão
automática com retorno “blocked by policy”, sem motivo mais específico. Essas
pastas continuam intactas; não foi usado outro shell/linguagem para contornar.
