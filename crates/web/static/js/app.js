/*
 * Pages du serveur ADN : dépôt de fichier, suivi des tâches, affichage des résultats.
 *
 * Tout texte venu du serveur entre dans la page par `textContent` : jamais comme balises.
 */
window.ADN = (function () {
    "use strict";

    const numberFormat = new Intl.NumberFormat("fr-FR");
    const number = (value) => (Number.isFinite(value) ? numberFormat.format(value) : "—");
    const decimal = (value, digits) =>
        Number.isFinite(value)
            ? value.toLocaleString("fr-FR", { maximumFractionDigits: digits })
            : "—";

    function bytes(value) {
        if (!Number.isFinite(value)) return "—";
        const units = ["octets", "Kio", "Mio", "Gio"];
        let size = value;
        let rank = 0;
        while (size >= 1024 && rank < units.length - 1) {
            size /= 1024;
            rank += 1;
        }
        const shown = rank === 0 ? size : size.toLocaleString("fr-FR", { maximumFractionDigits: 1 });
        return `${shown} ${units[rank]}`;
    }

    function seconds(value) {
        if (!Number.isFinite(value)) return "—";
        return value < 1 ? `${Math.round(value * 1000)} ms` : `${decimal(value, 2)} s`;
    }

    function when(text) {
        const date = new Date(text);
        return Number.isNaN(date.getTime()) ? text : date.toLocaleString("fr-FR");
    }

    const KIND = { encode: "Encodage", decode: "Décodage" };
    const STATE = { queued: "en attente", running: "en cours", complete: "terminée", failed: "échouée" };
    const COMPRESSION = { None: "aucune", Lz4: "LZ4", Zstd: "zstd" };
    const SCHEME = {
        fountain: "DNA Fountain",
        strands: "brins codés",
        goldman: "Goldman et al., 2013",
        grass: "Grass et al., 2015",
        ldpc: "code convolutif et LDPC",
    };
    const SCHEME_HELP = {
        fountain: "Sans adresse, marge de perte réglable : le schéma de l'article de référence.",
        strands: "Brins adressés, deux niveaux de Reed-Solomon : le plus économe pour les petits fichiers.",
        goldman: "Code ternaire de Huffman, jamais deux bases identiques de suite, chaque base lue dans quatre brins. " +
            "Environ 2 Mo au plus, GC non garanti par brin.",
        grass: "Reed-Solomon sur le corps à 47 éléments, un symbole par groupe de trois bases : le plus robuste aux substitutions, " +
            "le moins dense.",
        ldpc: "Code convolutif par brin, lu sur le treillis à décisions souples, et LDPC par bloc, " +
            "entrelacés : le plus tolérant aux erreurs de lecture et aux brins manquants. " +
            "Moins dense que les brins codés.",
    };

    function el(tag, className, text) {
        const node = document.createElement(tag);
        if (className) node.className = className;
        if (text !== undefined) node.textContent = text;
        return node;
    }

    function show(id) {
        document.getElementById(id).hidden = false;
    }

    function hide(id) {
        document.getElementById(id).hidden = true;
    }

    function message(kind, text) {
        const box = document.getElementById("message");
        box.hidden = false;
        box.className = `message ${kind}`;
        box.textContent = text;
    }

    function markNavigation() {
        const path = window.location.pathname.replace(/\/$/, "") || "/";
        document.querySelectorAll("nav a[data-nav]").forEach((link) => {
            if (link.dataset.nav === path) {
                link.classList.add("active");
                link.setAttribute("aria-current", "page");
            }
        });
    }

    /* Dépôt de fichier */
    function uploadLimit() {
        return Number(document.querySelector('meta[name="adn-upload-limit"]').content);
    }

    function watchDropZone(zone, input, onFile) {
        const choose = () => input.click();
        zone.addEventListener("click", choose);
        zone.addEventListener("keydown", (event) => {
            if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                choose();
            }
        });
        zone.addEventListener("dragover", (event) => {
            event.preventDefault();
            zone.classList.add("dragover");
        });
        zone.addEventListener("dragleave", () => zone.classList.remove("dragover"));
        zone.addEventListener("drop", (event) => {
            event.preventDefault();
            zone.classList.remove("dragover");
            const file = event.dataTransfer.files[0];
            if (file) {
                input.files = event.dataTransfer.files;
                onFile(file);
            }
        });
        input.addEventListener("change", () => {
            const file = input.files[0];
            if (file) onFile(file);
        });
    }

    /* Tâches */
    async function describe(response) {
        try {
            const body = await response.json();
            return body.error || `requête refusée (${response.status})`;
        } catch (error) {
            return `requête refusée (${response.status})`;
        }
    }

    async function startJob(url, file, fields, hooks) {
        const body = new FormData();
        body.append("file", file);
        Object.entries(fields).forEach(([name, value]) => {
            if (value !== "" && value !== null && value !== undefined) body.append(name, value);
        });
        const response = await fetch(url, { method: "POST", body });
        if (!response.ok) throw new Error(await describe(response));
        const accepted = await response.json();
        await follow(accepted.job_id, hooks);
    }

    async function follow(jobId, hooks) {
        for (;;) {
            const response = await fetch(`/api/jobs/${jobId}`);
            if (!response.ok) throw new Error(await describe(response));
            const job = await response.json();
            if (job.status === "complete") {
                hooks.onDone(job);
                return;
            }
            if (job.status === "failed") throw new Error(job.error || "la tâche a échoué");
            hooks.onStage(job.stage || "en attente");
            await new Promise((resolve) => setTimeout(resolve, 400));
        }
    }

    function fillStats(rows) {
        const list = document.getElementById("stats");
        list.textContent = "";
        rows.forEach(([label, value]) => {
            const cell = el("div");
            cell.append(el("dt", "", label), el("dd", "", value));
            list.append(cell);
        });
    }

    /* Page d'encodage */
    function initEncode() {
        const form = document.getElementById("form");
        const zone = document.getElementById("dropzone");
        const input = document.getElementById("file");
        const label = document.getElementById("file-label");
        const submit = document.getElementById("submit");
        const scheme = document.getElementById("scheme");
        const compression = document.getElementById("compression");
        const marginField = document.getElementById("redundancy-field");
        const slider = document.getElementById("redundancy");
        const marginValue = document.getElementById("redundancy-value");
        const marginHelp = document.getElementById("redundancy-help");
        let selected = null;

        const describeMargin = () => {
            const margin = Number(slider.value);
            marginValue.value = margin.toLocaleString("fr-FR", { maximumFractionDigits: 2 });
            marginHelp.textContent =
                `L'archive survit à la perte aléatoire de ${decimal((1 - 1 / margin) * 100, 0)} % de ses oligos. ` +
                "Une marge plus haute allonge l'archive.";
        };
        const followScheme = () => {
            const fountain = scheme.value === "fountain";
            marginField.hidden = !fountain;
            if (fountain) describeMargin();
            const help = document.getElementById("scheme-help");
            if (help) help.textContent = SCHEME_HELP[scheme.value] || "";
        };
        slider.addEventListener("input", describeMargin);
        scheme.addEventListener("change", followScheme);
        followScheme();

        watchDropZone(zone, input, (file) => {
            if (file.size > uploadLimit()) {
                selected = null;
                submit.disabled = true;
                zone.classList.remove("has-file");
                label.textContent = "Glissez un fichier ici, ou cliquez pour le choisir";
                message("error", `${file.name} dépasse la limite de ${bytes(uploadLimit())}.`);
                return;
            }
            hide("message");
            selected = file;
            label.textContent = `${file.name} (${bytes(file.size)})`;
            zone.classList.add("has-file");
            submit.disabled = false;
        });

        form.addEventListener("submit", async (event) => {
            event.preventDefault();
            if (!selected) return;
            hide("result");
            hide("message");
            show("progress");
            setStage("envoi du fichier");
            submit.disabled = true;
            try {
                await startJob(
                    "/api/encode",
                    selected,
                    {
                        scheme: scheme.value,
                        compression: compression.value,
                        redundancy: scheme.value === "fountain" ? slider.value : "",
                    },
                    {
                        onStage: setStage,
                        onDone: (job) => {
                            hide("progress");
                            showEncodeResult(job);
                            submit.disabled = false;
                        },
                    }
                );
            } catch (error) {
                hide("progress");
                message("error", error.message);
                submit.disabled = false;
            }
        });
    }

    function setStage(text) {
        document.getElementById("stage").textContent = text;
    }

    function showEncodeResult(job) {
        const stats = job.stats || {};
        const compressed =
            stats.compression && stats.compression !== "None"
                ? `, compressé en ${bytes(stats.body_bytes)}`
                : "";
        fillStats([
            ["Archive", job.file_name],
            ["Fichier d'origine", `${bytes(stats.original_bytes)}${compressed}`],
            ["Taille de l'archive", bytes(job.size)],
            ["Schéma", SCHEME[stats.scheme] || stats.scheme || "—"],
            ["Oligos", number(stats.oligos)],
            ["Nucléotides", number(stats.nucleotides)],
            ["Densité", `${decimal(stats.bits_per_nucleotide, 3)} bits par nucléotide`],
            ["Encodage", seconds(stats.encode_seconds)],
        ]);
        document.getElementById("sha256").textContent = job.sha256;
        const download = document.getElementById("download");
        download.href = job.download_url;
        download.setAttribute("download", job.file_name);
        show("result");
    }

    /* Page de décodage */
    function initDecode() {
        const form = document.getElementById("form");
        const zone = document.getElementById("dropzone");
        const input = document.getElementById("file");
        const label = document.getElementById("file-label");
        const submit = document.getElementById("submit");
        const expected = document.getElementById("expected");
        let selected = null;

        watchDropZone(zone, input, (file) => {
            if (file.size > uploadLimit()) {
                selected = null;
                submit.disabled = true;
                zone.classList.remove("has-file");
                label.textContent = "Glissez une archive FASTA ici, ou cliquez pour la choisir";
                message("error", `${file.name} dépasse la limite de ${bytes(uploadLimit())}.`);
                return;
            }
            hide("message");
            selected = file;
            label.textContent = `${file.name} (${bytes(file.size)})`;
            zone.classList.add("has-file");
            submit.disabled = false;
        });

        form.addEventListener("submit", async (event) => {
            event.preventDefault();
            if (!selected) return;
            hide("result");
            hide("message");
            show("progress");
            setStage("envoi de l'archive");
            submit.disabled = true;
            try {
                await startJob("/api/decode", selected, { expected_sha256: expected.value.trim() }, {
                    onStage: setStage,
                    onDone: (job) => {
                        hide("progress");
                        showDecodeResult(job);
                        submit.disabled = false;
                    },
                });
            } catch (error) {
                hide("progress");
                message("error", error.message);
                submit.disabled = false;
            }
        });
    }

    function showDecodeResult(job) {
        const stats = job.stats || {};
        const readings = [
            number(stats.reads_total),
            stats.corrected ? `dont ${number(stats.corrected)} corrigée(s)` : "",
            stats.uncorrectable ? `${number(stats.uncorrectable)} illisible(s)` : "",
        ]
            .filter(Boolean)
            .join(", ");
        fillStats([
            ["Fichier restitué", job.file_name],
            ["Taille", bytes(job.size)],
            ["Schéma de l'archive", SCHEME[stats.scheme] || stats.scheme || "—"],
            ["Compression", COMPRESSION[stats.compression] || stats.compression || "—"],
            ["Lectures", readings],
            ["Décodage", seconds(stats.decode_seconds)],
        ]);
        document.getElementById("sha256").textContent = job.sha256;
        const download = document.getElementById("download");
        download.href = job.download_url;
        download.setAttribute("download", job.file_name);
        show("result");
    }

    /* Liste des tâches */
    function jobRow(job) {
        const row = el("tr");
        row.append(el("td", "", KIND[job.kind] || job.kind));

        const name = el("td", "name", job.file_name || job.source_name || "—");
        name.title = job.source_name || "";
        row.append(name);

        const state = el("td");
        state.append(el("span", `badge ${job.status}`, STATE[job.status] || job.status));
        if (job.status === "queued" || job.status === "running") {
            state.append(el("span", "muted", ` ${job.stage || ""}`));
        }
        if (job.status === "failed" && job.error) {
            state.append(el("div", "error", job.error));
        }
        row.append(state);

        row.append(el("td", "", bytes(job.size)));
        row.append(el("td", "", when(job.created_at)));

        const actions = el("td");
        const buttons = el("div", "actions");
        if (job.status === "complete" && job.download_url) {
            const link = el("a", "button secondary", "Télécharger");
            link.href = job.download_url;
            buttons.append(link);
        }
        if (job.status === "complete" || job.status === "failed") {
            const remove = el("button", "button danger", "Supprimer");
            remove.addEventListener("click", () => removeJob(job.id));
            buttons.append(remove);
        }
        actions.append(buttons);
        row.append(actions);
        return row;
    }

    async function renderJobs() {
        const container = document.getElementById("jobs");
        if (!container) return;
        try {
            const response = await fetch("/api/jobs");
            if (!response.ok) throw new Error(await describe(response));
            const jobs = (await response.json()).jobs;
            container.textContent = "";
            if (jobs.length === 0) {
                container.append(el("p", "muted", "Aucune tâche pour l'instant."));
                return;
            }
            const table = el("table");
            const head = el("tr");
            ["Tâche", "Fichier", "État", "Taille", "Créée", "Actions"].forEach((title) => head.append(el("th", "", title)));
            table.append(el("thead"), el("tbody"));
            table.tHead.append(head);
            jobs.forEach((job) => table.tBodies[0].append(jobRow(job)));
            container.append(table);
            // La liste se rafraîchit tant qu'une tâche tourne.
            if (jobs.some((job) => job.status === "queued" || job.status === "running")) {
                setTimeout(renderJobs, 1000);
            }
        } catch (error) {
            container.textContent = "";
            container.append(el("p", "error", `Liste indisponible : ${error.message}`));
        }
    }

    async function removeJob(id) {
        if (!window.confirm("Supprimer cette tâche et son résultat ?")) return;
        const response = await fetch(`/api/jobs/${id}`, { method: "DELETE" });
        if (!response.ok && response.status !== 404) {
            window.alert(`Suppression impossible : ${await describe(response)}`);
        }
        renderJobs();
    }

    markNavigation();
    const page = document.body.dataset.page;
    if (page === "encode") initEncode();
    if (page === "decode") initDecode();

    return { renderJobs };
})();
