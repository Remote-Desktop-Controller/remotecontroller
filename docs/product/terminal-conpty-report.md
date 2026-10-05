# Diagnóstico nativo Windows — ConPTY

O teste real capturou somente ESC[6n: o processo aguardava resposta à consulta de
posição do cursor. portable-pty 0.9.0 usa flags de emulador gráfico. A aplicação
é um fluxo VT headless, portanto o patch usa flags=0 documentado pela Microsoft.
Original src/win/psuedocon.rs SHA256:
297D0622D7B0401708F1CE3B92076632DD5A0C188B4A8D09B2F4891190B0E392.

Depois dessa correção o teste avançou e revelou outro problema real: cmd.exe
interpreta o prefixo canônico Windows \\?\C:\ como UNC, troca para C:\Windows e
perde o cwd pedido. O manager valida o path canônico primeiro e converte somente
VerbatimDisk para o path DOS equivalente. UNC com cmd.exe é recusado explicitamente.

RED: terminal-native-debug.log e terminal-conpty-green.log (nome de tentativa,
não declaração de sucesso). O GREEN final é registrado separadamente.

[CreatePseudoConsole](https://learn.microsoft.com/en-us/windows/console/createpseudoconsole)
documenta flags=0, UTF-8/VT e o risco de hang ao não responder ao cursor herdado.
