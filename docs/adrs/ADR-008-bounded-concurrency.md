# ADR-008 — Admissão e concorrência limitadas
Status: aceito. Slot de admissão cobre toda a vida de um job, limitando tanto
fila quanto operações ativas. Semaphores separados por FileRead/FileWrite/Process/
Database/Cpu/Background. mpsc finito para progress; JoinSet para clientes.
Busy é erro explícito, não crescimento infinito. Defaults são configuráveis.
