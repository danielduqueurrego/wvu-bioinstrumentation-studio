<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { confirm, open } from '@tauri-apps/plugin-dialog';
  import { revealItemInDir } from '@tauri-apps/plugin-opener';
  import { EditorState } from '@codemirror/state';
  import { EditorView, drawSelection, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view';
  import { bracketMatching, foldGutter, indentOnInput, syntaxHighlighting, defaultHighlightStyle } from '@codemirror/language';
  import { cpp } from '@codemirror/lang-cpp';
  import { history, defaultKeymap, historyKeymap, indentWithTab } from '@codemirror/commands';
  import { highlightSelectionMatches, searchKeymap } from '@codemirror/search';
  import { autocompletion, completionKeymap } from '@codemirror/autocomplete';
  import { firmwareCompatibilityMessage, firmwareControls, type FirmwareCompatibility } from '$lib/firmware-controls';

  type Template = {
    kind: TemplateKind;
    name: string;
    verification_kind: 'wvu_protocol_reference' | 'non_wvu';
    description: string;
  };
  type TemplateKind = 'blank_uno_r4_wifi' | 'a0_acquisition_example' | 'wvu_protocol_reference' | 'safe_digital_output' | 'serial_diagnostic';
  type Identity = { protocol_version: string; firmware_build: number; device_id: number };
  type Metadata = {
    project_name: string;
    source_filename: string;
    selected_com_port?: string;
    template_origin: TemplateKind;
    verification_kind: 'wvu_protocol_reference' | 'non_wvu';
    notes?: string;
    last_successful_compile_utc?: string;
    last_successful_upload_utc?: string;
    last_verified_firmware_identity?: Identity;
  };
  type Project = { project_folder: string; source_path: string; metadata_path: string; metadata: Metadata; source: string; source_hash: string };
  type Board = { port: string; name: string; fqbn: string; serial_number?: string; matching_boards?: string[] };
  type Diagnostic = { severity: string; file?: string; line?: number; column?: number; message: string };
  type CommandLog = { command: string[]; exit_code?: number; stdout: string; stderr: string; duration_ms: number; canceled: boolean };
  type Failure = { category: string; stage: string; title: string; explanation: string; recommended_action: string; technical_details: string };
  type Verification = { declared_kind: string; compatible: boolean; protocol_version?: string; identity?: Identity; bytes_received?: number; valid_frames?: number; crc_failures?: number; explanation: string };
  type Job = {
    id: number; kind: string; stage: string; active: boolean; project_folder?: string; original_port?: string; bootloader_port?: string; final_port?: string; board_serial?: string;
    message: string; compile_usage?: { sketch_bytes?: number; sketch_percent?: number; ram_bytes?: number; ram_percent?: number };
    diagnostics: Diagnostic[]; compile_log?: CommandLog; upload_log?: CommandLog; verification?: Verification; failure?: Failure; log_path?: string;
  };
  type WorkflowStatus = { compatibility: Compatibility; job?: Job; last_compile?: Job; last_upload?: Job; last_failure?: Failure };
  type Compatibility = FirmwareCompatibility;
  type Environment = { cli_path?: string; cli_version?: string; uno_r4_core_version?: string; expected_fqbn: string; boards: Board[]; ready: boolean; problem?: string };

  let editorHost: HTMLDivElement;
  let editor: EditorView | undefined;
  let editorSource = '';
  let project: Project | undefined;
  let templates: Template[] = [];
  let environment: Environment = { expected_fqbn: 'arduino:renesas_uno:unor4wifi', boards: [], ready: false };
  let workflow: WorkflowStatus = { compatibility: 'unknown' };
  let parentFolder = '';
  let projectName = 'MyUnoR4Project';
  let selectedTemplate: TemplateKind = 'blank_uno_r4_wifi';
  let selectedPort = '';
  let notes = '';
  let saveAsParent = '';
  let saveAsName = '';
  let recents: string[] = [];
  let fontSize = 14;
  let statusMessage = 'Create or open a single-file Arduino project to begin.';
  let polling = false;
  let sourceAtSave = '';
  let outputFilter = '';

  $: dirty = Boolean(project) && editorSource !== sourceAtSave;
  $: activeJob = workflow.job?.active ?? false;
  $: currentJob = workflow.job ?? workflow.last_upload ?? workflow.last_compile;
  $: controls = firmwareControls({ hasProject: Boolean(project), unsavedChanges: dirty, activeJob, hasCurrentCompile: Boolean(workflow.last_compile), selectedPort });
  $: compileDisabled = !controls.compileEnabled;
  $: uploadDisabled = !controls.uploadEnabled;
  $: compatibilityText = firmwareCompatibilityMessage(workflow.compatibility);
  $: selectedTemplateInfo = templates.find((template) => template.kind === selectedTemplate);
  $: outputText = jobOutput(currentJob);
  $: filteredOutput = outputFilter.trim()
    ? outputText.split('\n').filter((line) => line.toLowerCase().includes(outputFilter.toLowerCase())).join('\n')
    : outputText;

  function jobOutput(job: Job | undefined) {
    if (!job) return 'No compile or upload output yet.';
    const lines = [
      `Stage: ${job.stage}`, `Status: ${job.message}`,
      job.failure ? `${job.failure.title}: ${job.failure.explanation}\nRecommended action: ${job.failure.recommended_action}\n${job.failure.technical_details}` : '',
      job.compile_log ? formatLog('Compile', job.compile_log) : '',
      job.upload_log ? formatLog('Upload', job.upload_log) : ''
    ];
    return lines.filter(Boolean).join('\n\n');
  }

  function formatLog(label: string, log: CommandLog) {
    return `${label} command:\n${log.command.join(' ')}\nExit: ${log.exit_code ?? 'not available'}; duration: ${log.duration_ms} ms${log.canceled ? '; canceled' : ''}\n\nstdout:\n${log.stdout || '(none)'}\n\nstderr:\n${log.stderr || '(none)'}`;
  }

  function setEditorSource(source: string) {
    editorSource = source;
    if (!editor) return;
    const current = editor.state.doc.toString();
    if (current !== source) editor.dispatch({ changes: { from: 0, to: current.length, insert: source } });
  }

  function createEditor() {
    if (!editorHost) return;
    editor = new EditorView({
      state: EditorState.create({
        doc: editorSource,
        extensions: [
          lineNumbers(),
          highlightActiveLineGutter(),
          history(),
          foldGutter(),
          drawSelection(),
          EditorState.allowMultipleSelections.of(true),
          indentOnInput(),
          syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
          bracketMatching(),
          highlightActiveLine(),
          highlightSelectionMatches(),
          autocompletion(),
          cpp(),
          EditorView.lineWrapping,
          EditorView.updateListener.of((update) => {
            if (update.docChanged) editorSource = update.state.doc.toString();
          }),
          keymap.of([
            { key: 'Mod-s', run: () => { void saveProject(); return true; } },
            { key: 'Mod-Shift-s', run: () => { void saveAsProject(); return true; } },
            indentWithTab,
            ...defaultKeymap,
            ...historyKeymap,
            ...searchKeymap,
            ...completionKeymap
          ])
        ]
      }),
      parent: editorHost
    });
  }

  async function refreshEnvironment() {
    try {
      environment = await invoke<Environment>('firmware_environment');
      if (!selectedPort || !environment.boards.some((board) => board.port === selectedPort)) {
        selectedPort = project?.metadata.selected_com_port ?? environment.boards[0]?.port ?? '';
      }
      statusMessage = environment.ready
        ? `Arduino CLI ${environment.cli_version ?? ''}; Renesas UNO core ${environment.uno_r4_core_version ?? ''}.`
        : environment.problem ?? 'Arduino CLI environment is not ready. Editing and saving remain available.';
    } catch (error) {
      statusMessage = `Could not inspect the Arduino environment: ${String(error)}`;
    }
  }

  async function refreshWorkflow() {
    if (polling) return;
    polling = true;
    try {
      workflow = await invoke<WorkflowStatus>('get_firmware_workflow_status');
    } catch (error) {
      statusMessage = `Could not read firmware workflow status: ${String(error)}`;
    } finally {
      polling = false;
    }
  }

  async function chooseFolder(target: 'parent' | 'open' | 'saveAs') {
    const result = await open({ directory: true, multiple: false, title: target === 'open' ? 'Open Arduino project folder' : 'Choose project parent folder' });
    if (!result || Array.isArray(result)) return;
    if (target === 'open') await openProject(result);
    else if (target === 'saveAs') saveAsParent = result;
    else parentFolder = result;
  }

  async function ensureDiscardOrSave(action: () => Promise<void>) {
    if (dirty) {
      const save = await confirm('This project has unsaved code changes. Save before continuing?', { title: 'Unsaved firmware changes', kind: 'warning', okLabel: 'Save', cancelLabel: 'Cancel' });
      if (!save) return;
      const saved = await saveProject();
      if (!saved) return;
    }
    await action();
  }

  async function createProject() {
    if (!parentFolder.trim()) {
      statusMessage = 'Choose a parent folder for the new project.';
      return;
    }
    await ensureDiscardOrSave(async () => {
      try {
        project = await invoke<Project>('create_firmware_project', {
          request: { parentFolder, projectName, template: selectedTemplate, notes: notes || null, overwriteConfirmed: false }
        });
        sourceAtSave = project.source;
        setEditorSource(project.source);
        notes = project.metadata.notes ?? '';
        selectedPort = project.metadata.selected_com_port ?? selectedPort;
        saveAsParent = parentFolder;
        saveAsName = project.metadata.project_name;
        statusMessage = `Created ${project.metadata.source_filename}. The template source is a project copy; repository templates remain unchanged.`;
      } catch (error) {
        statusMessage = `Create project error: ${String(error)}`;
      }
    });
  }

  async function openProject(folder: string) {
    await ensureDiscardOrSave(async () => {
      try {
        project = await invoke<Project>('open_firmware_project', { projectFolder: folder });
        sourceAtSave = project.source;
        setEditorSource(project.source);
        notes = project.metadata.notes ?? '';
        selectedPort = project.metadata.selected_com_port ?? selectedPort;
        parentFolder = folder.replace(/[\\/][^\\/]+$/, '');
        saveAsParent = parentFolder;
        saveAsName = project.metadata.project_name;
        statusMessage = `Opened ${project.metadata.source_filename}.`;
      } catch (error) {
        statusMessage = `Open project error: ${String(error)}`;
      }
    });
  }

  async function saveProject(): Promise<boolean> {
    if (!project) return false;
    try {
      project = await invoke<Project>('save_firmware_project', {
        request: { projectFolder: project.project_folder, source: editorSource, notes: notes || null, selectedComPort: selectedPort || null }
      });
      sourceAtSave = project.source;
      statusMessage = `Saved ${project.metadata.source_filename}.`;
      return true;
    } catch (error) {
      statusMessage = `Save error: ${String(error)}`;
      return false;
    }
  }

  async function saveAsProject() {
    if (!project) return;
    if (!saveAsParent.trim() || !saveAsName.trim()) {
      statusMessage = 'Choose a Save As parent folder and a valid project name.';
      return;
    }
    const allowed = await confirm(`Save a new project named “${saveAsName}”? Existing non-empty folders are never overwritten.`, { title: 'Save firmware project as', kind: 'warning', okLabel: 'Save As', cancelLabel: 'Cancel' });
    if (!allowed) return;
    try {
      project = await invoke<Project>('save_firmware_project_as', {
        request: { sourceProjectFolder: project.project_folder, destinationParentFolder: saveAsParent, destinationProjectName: saveAsName, source: editorSource, overwriteConfirmed: false }
      });
      sourceAtSave = project.source;
      statusMessage = `Saved a new project: ${project.project_folder}.`;
    } catch (error) {
      statusMessage = `Save As error: ${String(error)}`;
    }
  }

  async function restoreSavedSource() {
    if (!project || !dirty) return;
    const allowed = await confirm('Restore the last saved source? Unsaved editor changes will be discarded.', { title: 'Restore saved firmware', kind: 'warning', okLabel: 'Restore', cancelLabel: 'Cancel' });
    if (!allowed) return;
    try {
      const saved = await invoke<string>('restore_firmware_project_saved_source', { projectFolder: project.project_folder });
      sourceAtSave = saved;
      setEditorSource(saved);
      statusMessage = 'Restored the last saved source.';
    } catch (error) {
      statusMessage = `Restore source error: ${String(error)}`;
    }
  }

  async function compileProject() {
    if (!project || dirty || activeJob) return;
    try {
      workflow.job = await invoke<Job>('start_firmware_compile', { request: { projectFolder: project.project_folder, unsavedChanges: false } });
      statusMessage = `Compiling saved project ${project.metadata.source_filename} for ${environment.expected_fqbn}.`;
    } catch (error) {
      statusMessage = `Compile could not start: ${friendlyFailure(error)}`;
    }
  }

  async function uploadProject() {
    if (!project || dirty || activeJob || !selectedPort) return;
    const allowed = await confirm(
      `Upload ${project.metadata.source_filename} to ${selectedPort}? This replaces the current Arduino sketch. Student firmware may not be electrically safe merely because it compiles. No person or biomedical accessory may be connected.`,
      { title: 'Confirm Arduino upload', kind: 'warning', okLabel: 'Upload', cancelLabel: 'Cancel' }
    );
    if (!allowed) return;
    try {
      workflow.job = await invoke<Job>('start_firmware_upload', { request: { projectFolder: project.project_folder, port: selectedPort, unsavedChanges: false, confirmation: true } });
      statusMessage = 'Closing the production serial session and starting the selected-board upload workflow.';
    } catch (error) {
      statusMessage = `Upload could not start: ${friendlyFailure(error)}`;
    }
  }

  async function restoreReference() {
    if (!selectedPort || activeJob) return;
    const allowed = await confirm(
      `Restore the controlled WVU reference firmware on ${selectedPort}? This replaces the current Arduino sketch and will only report success after HELLO, CAPABILITIES, PONG, version, and identity verification.`,
      { title: 'Restore WVU reference firmware', kind: 'warning', okLabel: 'Restore firmware', cancelLabel: 'Cancel' }
    );
    if (!allowed) return;
    try {
      workflow.job = await invoke<Job>('restore_wvu_reference_firmware', { request: { port: selectedPort, confirmation: true } });
      statusMessage = 'Restoring the controlled repository reference firmware. Acquisition remains disabled until identity verification passes.';
    } catch (error) {
      statusMessage = `Reference restore could not start: ${friendlyFailure(error)}`;
    }
  }

  async function verifyReference() {
    if (!selectedPort || activeJob) return;
    try {
      const verification = await invoke<Verification>('verify_wvu_reference_firmware', { port: selectedPort });
      await refreshWorkflow();
      statusMessage = verification.compatible
        ? `Verified WVU protocol firmware on ${selectedPort}: ${verification.protocol_version ?? 'protocol version unavailable'}.`
        : verification.explanation;
    } catch (error) {
      statusMessage = `Reference verification failed: ${friendlyFailure(error)}`;
      await refreshWorkflow();
    }
  }

  async function cancelJob() {
    try {
      workflow = await invoke<WorkflowStatus>('cancel_firmware_job');
      statusMessage = 'Cancellation was requested for the active Arduino CLI child process.';
    } catch (error) {
      statusMessage = `Cancel error: ${friendlyFailure(error)}`;
    }
  }

  async function copyOutput() {
    try {
      await navigator.clipboard.writeText(outputText);
      statusMessage = 'Build and upload output copied to the clipboard.';
    } catch (error) {
      statusMessage = `Clipboard error: ${String(error)}`;
    }
  }

  async function openProjectFolder() {
    if (!project) return;
    try {
      await revealItemInDir(project.project_folder);
    } catch (error) {
      statusMessage = `Could not open the project folder: ${String(error)}`;
    }
  }

  async function openWorkflowLog() {
    if (!currentJob?.log_path) return;
    try {
      await revealItemInDir(currentJob.log_path);
    } catch (error) {
      statusMessage = `Could not open the workflow log: ${String(error)}`;
    }
  }

  function navigateDiagnostic(diagnostic: Diagnostic) {
    if (!editor || !diagnostic.line) return;
    try {
      const line = editor.state.doc.line(diagnostic.line);
      editor.dispatch({ selection: { anchor: line.from }, scrollIntoView: true });
      editor.focus();
    } catch {
      statusMessage = `The reported line ${diagnostic.line} is outside the current source file.`;
    }
  }

  function friendlyFailure(error: unknown) {
    const message = String(error);
    try {
      const failure = JSON.parse(message) as Failure;
      return `${failure.title}: ${failure.explanation} ${failure.recommended_action}`;
    } catch {
      return message;
    }
  }

  onMount(() => {
    createEditor();
    void (async () => {
      try {
        templates = await invoke<Template[]>('list_firmware_templates');
        recents = await invoke<string[]>('list_recent_firmware_projects');
      } catch (error) {
        statusMessage = `Could not load firmware workspace: ${String(error)}`;
      }
      await refreshEnvironment();
      await refreshWorkflow();
    })();
    const timer = window.setInterval(() => {
      void refreshWorkflow();
      if (workflow.job?.active) void refreshEnvironment();
    }, 500);
    const beforeUnload = (event: BeforeUnloadEvent) => {
      if (!dirty) return;
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener('beforeunload', beforeUnload);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener('beforeunload', beforeUnload);
      editor?.destroy();
    };
  });
</script>

<section class="firmware-workspace" aria-labelledby="firmware-title">
  <div class="firmware-heading">
    <div>
      <h2 id="firmware-title">Firmware workspace</h2>
      <p>One local Arduino <code>.ino</code> file per project. Editing and saving work offline; compiling and uploading require Arduino CLI and the installed UNO R4 core.</p>
    </div>
    <span class:compatible={workflow.compatibility === 'wvu_protocol_compatible'} class:incompatible={workflow.compatibility !== 'wvu_protocol_compatible'} class="compatibility" role="status">{compatibilityText}</span>
  </div>

  <section class="firmware-panel project-controls" aria-labelledby="project-title">
    <h3 id="project-title">Project</h3>
    <div class="firmware-grid">
      <label>Project parent folder
        <div class="input-action"><input bind:value={parentFolder} placeholder="Choose a folder" /><button type="button" onclick={() => chooseFolder('parent')}>Choose</button></div>
      </label>
      <label>Project name <input bind:value={projectName} maxlength="63" pattern="[A-Za-z][A-Za-z0-9_]*" placeholder="MyUnoR4Project" /></label>
      <label>Template
        <select bind:value={selectedTemplate}>
          {#each templates as template}<option value={template.kind}>{template.name}</option>{/each}
        </select>
      </label>
      <div class="field-action"><span>New project</span><button class="gold" type="button" onclick={createProject}>New</button></div>
      <div class="field-action"><span>Open a project folder</span><button type="button" onclick={() => chooseFolder('open')}>Open</button></div>
      <div class="field-action"><span>Saved source</span><button type="button" onclick={saveProject} disabled={!project || !dirty}>Save</button></div>
      <div class="field-action"><span>Project location</span><button type="button" onclick={openProjectFolder} disabled={!project}>Open folder</button></div>
    </div>
    {#if selectedTemplateInfo}<p class="help">{selectedTemplateInfo.description} {selectedTemplateInfo.verification_kind === 'non_wvu' ? 'It is not WVU protocol firmware; Acquisition remains unavailable after upload until the controlled reference is restored.' : 'It is copied byte-for-byte from the controlled repository firmware.'}</p>{/if}
    {#if project}<p class="project-path" title={project.project_folder}><strong>{project.metadata.source_filename}</strong> — {project.project_folder} {#if dirty}<span class="dirty" aria-label="Unsaved changes">Unsaved changes</span>{:else}<span class="saved">Saved</span>{/if}</p>{/if}
    {#if recents.length}
      <div class="recent"><span>Recent projects</span>{#each recents as recent}<button type="button" title={recent} onclick={() => openProject(recent)}>{recent}</button>{/each}</div>
    {/if}
  </section>

  <section class="firmware-panel save-as" aria-label="Save As controls">
    <div class="firmware-grid compact-grid">
      <label>Save As parent folder <div class="input-action"><input bind:value={saveAsParent} placeholder="Choose a folder" /><button type="button" onclick={() => chooseFolder('saveAs')}>Choose</button></div></label>
      <label>New project name <input bind:value={saveAsName} maxlength="63" placeholder="NewProjectName" /></label>
      <div class="field-action"><span>Save As</span><button type="button" onclick={saveAsProject} disabled={!project}>Save As</button></div>
      <div class="field-action"><span>Discard editor changes</span><button type="button" onclick={restoreSavedSource} disabled={!dirty}>Restore saved</button></div>
      <label>Editor font size <input type="number" min="11" max="24" bind:value={fontSize} oninput={() => editor && (editor.dom.style.fontSize = `${fontSize}px`)} /></label>
    </div>
  </section>

  <section class="editor-build-layout">
    <div class="editor-panel firmware-panel">
      <div class="editor-heading"><h3>Arduino / C++ editor</h3><span>{dirty ? 'Unsaved changes' : project ? 'Saved' : 'No project open'}</span></div>
      <div class="editor-host" bind:this={editorHost} aria-label="Arduino source code editor"></div>
      <p class="help">Ctrl+S saves; Ctrl+Shift+S opens Save As; Ctrl+F finds; Ctrl+H replaces. Code is saved as UTF-8 without hidden transformation.</p>
    </div>

    <aside class="environment-panel firmware-panel" aria-labelledby="environment-title">
      <div class="panel-heading"><h3 id="environment-title">Firmware environment</h3><button type="button" onclick={refreshEnvironment}>Refresh environment</button></div>
      <dl>
        <dt>Arduino CLI</dt><dd title={environment.cli_path}>{environment.cli_path ?? 'Not found'} {environment.cli_version ? `(${environment.cli_version})` : ''}</dd>
        <dt>UNO R4 core</dt><dd>{environment.uno_r4_core_version ?? 'Not installed'}</dd>
        <dt>Expected FQBN</dt><dd>{environment.expected_fqbn}</dd>
        <dt>Readiness</dt><dd>{environment.ready ? 'Ready' : environment.problem ?? 'Not ready'}</dd>
      </dl>
      <label>Selected UNO R4 WiFi
        <select bind:value={selectedPort} disabled={activeJob}>
          <option value="">Select a detected UNO R4 WiFi</option>
          {#each environment.boards as board}<option value={board.port}>{board.name} — {board.port}{board.serial_number ? ` (${board.serial_number})` : ''}</option>{/each}
        </select>
      </label>
      <div class="action-stack">
        <button type="button" onclick={refreshEnvironment} disabled={activeJob}>Refresh boards</button>
        <button type="button" onclick={verifyReference} disabled={!selectedPort || activeJob}>Verify WVU firmware</button>
        <button class="gold" type="button" onclick={compileProject} disabled={compileDisabled}>Compile saved project</button>
        <button type="button" onclick={uploadProject} disabled={uploadDisabled}>Upload selected project</button>
        <button class="restore" type="button" onclick={restoreReference} disabled={!selectedPort || activeJob}>Restore WVU reference firmware</button>
        {#if activeJob}<button class="stop" type="button" onclick={cancelJob}>Cancel {workflow.job?.kind}</button>{/if}
      </div>
      <p class="teaching-warning">Teaching use only. Compilation does not establish electrical safety. Upload is blocked while acquisition or recording owns the session.</p>
      {#if workflow.last_upload?.verification}
        <p class="verification">{workflow.last_upload.verification.explanation}</p>
      {/if}
    </aside>
  </section>

  <section class="firmware-panel diagnostics-output" aria-labelledby="build-output-title">
    <div class="panel-heading"><h3 id="build-output-title">Compile and upload diagnostics</h3><div class="button-pair"><button type="button" onclick={copyOutput}>Copy diagnostics</button>{#if currentJob?.log_path}<button type="button" title={currentJob.log_path} onclick={openWorkflowLog}>Open log</button>{/if}</div></div>
    {#if currentJob}
      <div class="job-progress" aria-live="polite"><strong>{currentJob.kind}</strong> — {currentJob.stage}: {currentJob.message} {#if currentJob.original_port}<span>Original: {currentJob.original_port}; bootloader: {currentJob.bootloader_port ?? 'not observed'}; final: {currentJob.final_port ?? 'pending'}.</span>{/if}</div>
      {#if currentJob.compile_usage}<p>Program: {currentJob.compile_usage.sketch_bytes ?? '—'} bytes ({currentJob.compile_usage.sketch_percent ?? '—'}%). Data: {currentJob.compile_usage.ram_bytes ?? '—'} bytes ({currentJob.compile_usage.ram_percent ?? '—'}%).</p>{/if}
      {#if currentJob.diagnostics.length}
        <ul class="diagnostic-list">{#each currentJob.diagnostics as diagnostic}<li class:warning={diagnostic.severity === 'warning'} class:error={diagnostic.severity === 'error'}><button type="button" onclick={() => navigateDiagnostic(diagnostic)}>{diagnostic.severity}{diagnostic.line ? ` line ${diagnostic.line}` : ''}: {diagnostic.message}</button></li>{/each}</ul>
      {/if}
    {/if}
    <label>Filter output <input bind:value={outputFilter} placeholder="Filter warnings, errors, or lines" /></label>
    <div class="output-log" role="region" aria-label="Build output"><pre>{filteredOutput}</pre></div>
  </section>

  <p class="firmware-status" aria-live="polite">{statusMessage}</p>
</section>

<style>
  .firmware-workspace { min-width: 0; width: 100%; }
  .firmware-heading, .panel-heading, .editor-heading { display: flex; flex-wrap: wrap; gap: .75rem; align-items: start; justify-content: space-between; }
  .firmware-heading h2 { margin: 0; } .firmware-heading p { margin: .35rem 0 0; max-width: 70ch; }
  .compatibility { max-width: min(100%, 38rem); border-radius: .3rem; padding: .55rem .7rem; font-weight: 650; overflow-wrap: anywhere; background: #fff4dd; border: 1px solid #a76b00; color: #553600; }
  .compatibility.compatible { background: #eaf6ee; border-color: #27753c; color: #174f27; } .compatibility.incompatible { background: #fff4dd; }
  .firmware-panel { min-width: 0; margin: 1rem 0; padding: clamp(.8rem, 2vw, 1rem); background: #fff; border: 1px solid #d7dde2; border-radius: .35rem; }
  .firmware-panel h3 { margin: 0 0 .8rem; } .firmware-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 15rem), 1fr)); gap: .75rem; align-items: end; }
  label, .field-action { min-width: 0; display: grid; gap: .35rem; font-weight: 650; } .field-action > span { font-size: .9rem; }
  input, select { width: 100%; min-width: 0; min-height: 2.4rem; border: 1px solid #8493a0; border-radius: .25rem; background: #fff; padding: .35rem .5rem; }
  button { min-height: 2.45rem; border: 1px solid #002855; border-radius: .3rem; background: #fff; color: #002855; padding: .5rem .7rem; font: inherit; font-weight: 650; cursor: pointer; overflow-wrap: anywhere; } button:not(:disabled):hover { background: #002855; color: #fff; } button:disabled { cursor: not-allowed; opacity: .55; } button:focus-visible, input:focus-visible, select:focus-visible, pre:focus-visible { outline: 3px solid #EEAA00; outline-offset: 2px; }
  button.gold { background: #EEAA00; color: #17222e; } button.restore { border-color: #855a00; color: #684600; } button.stop { background: #9d2424; border-color: #761a1a; color: #fff; }
  .input-action { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: .45rem; } .help { margin: .65rem 0 0; color: #4b5965; } .project-path, dd, .recent button { overflow-wrap: anywhere; word-break: break-word; }
  .dirty { color: #8b1515; font-weight: 700; margin-left: .5rem; } .saved { color: #176c33; font-weight: 700; margin-left: .5rem; } .recent { margin-top: .8rem; display: grid; gap: .35rem; } .recent button { text-align: left; min-height: 2rem; }
  /* The environment panel remains a readable 304–400 px rail on wide desktops; the
     editor receives all remaining space instead of keeping both panels artificially narrow. */
  .editor-build-layout { display: grid; grid-template-columns: minmax(0, 1fr) minmax(19rem, 25rem); gap: 1rem; align-items: start; } .editor-panel, .environment-panel { min-width: 0; margin: 0; }
  .editor-host { min-width: 0; min-height: clamp(22rem, 53vh, 46rem); border: 1px solid #8493a0; border-radius: .25rem; overflow: auto; background: #fff; } :global(.editor-host .cm-editor) { min-height: clamp(22rem, 53vh, 46rem); height: 100%; font-family: Consolas, "Cascadia Code", monospace; font-size: 14px; } :global(.editor-host .cm-scroller) { overflow: auto; }
  dl { display: grid; grid-template-columns: minmax(7rem, auto) minmax(0, 1fr); gap: .4rem .6rem; margin: 0 0 1rem; } dt { font-weight: 700; } dd { min-width: 0; margin: 0; } .action-stack { display: grid; gap: .5rem; margin-top: .8rem; }
  .teaching-warning { border-left: 4px solid #EEAA00; background: #fff8e8; padding: .65rem; font-size: .92rem; } .verification { padding: .65rem; background: #eef4f8; overflow-wrap: anywhere; }
  .button-pair { display: flex; flex-wrap: wrap; gap: .5rem; } .job-progress { padding: .65rem; background: #eef4f8; border-left: 4px solid #002855; overflow-wrap: anywhere; } .job-progress span { display: block; margin-top: .35rem; } .diagnostic-list { padding-left: 1.3rem; } .diagnostic-list li.warning button { color: #7a5000; } .diagnostic-list li.error button { color: #8b1515; } .diagnostic-list button { min-height: unset; padding: .35rem; border: 0; text-align: left; } .diagnostic-list button:hover { background: #eef4f8; color: #002855; }
  .output-log { max-height: clamp(12rem, 35vh, 30rem); overflow: auto; border: 1px solid #d7dde2; background: #142334; color: #eef5fa; border-radius: .25rem; } .output-log pre { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; padding: .75rem; } .firmware-status { margin: 1rem 0 0; padding: .7rem; border: 1px solid #d7dde2; border-radius: .3rem; background: #fff; overflow-wrap: anywhere; }
  @media (max-width: 1050px) { .editor-build-layout { grid-template-columns: 1fr; } .environment-panel { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 17rem), 1fr)); gap: .8rem; } .environment-panel .panel-heading { grid-column: 1 / -1; } .environment-panel .action-stack, .environment-panel .teaching-warning, .environment-panel .verification { margin: 0; } }
  @media (max-width: 650px) { .firmware-heading, .panel-heading { align-items: stretch; } .panel-heading .button-pair { width: 100%; } .panel-heading button, .button-pair button { flex: 1 1 12rem; } .input-action { grid-template-columns: 1fr; } .compact-grid { grid-template-columns: 1fr; } dl { grid-template-columns: 1fr; gap: .15rem; } dd { margin-bottom: .4rem; } }
</style>
