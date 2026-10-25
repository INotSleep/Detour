import { useEffect, useState } from "react";
import { AppWindow, FolderSearch } from "lucide-react";
import { api, errorText, type Editor } from "../lib/api";
import { useStore } from "../lib/store";
import { useT } from "../i18n";
import { Button, Checkbox, Modal } from "./ui";

let found: Promise<Editor[]> | null = null;

/** The editors installed on this computer, looked up once. */
export function useEditors() {
  const [list, setList] = useState<Editor[]>([]);
  useEffect(() => {
    found ??= api.editors().catch(() => []);
    void found.then(setList);
  }, []);
  return list;
}

/** Opens a file on this computer in `editor`, or with the system's app for it. */
export async function openIn(path: string, editor: Editor | null) {
  await (editor ? api.openWith(path, editor.path) : api.openLocal(path));
}

/**
 * Which program to edit a file in, with the one remembered for "Edit" first.
 * `onPick` gets the program; with "remember" it becomes the one "Edit" uses.
 */
export function EditorChooser({ onPick, onClose }: { onPick: (editor: Editor) => void; onClose: () => void }) {
  const t = useT();
  const { settings, setSettings, toast } = useStore();
  const installed = useEditors();
  const [extra, setExtra] = useState<Editor[]>(settings.editor ? [settings.editor] : []);
  const list = [...extra, ...installed.filter((e) => !extra.some((x) => x.path.toLowerCase() === e.path.toLowerCase()))];
  const [chosen, setChosen] = useState<string | null>(settings.editor?.path ?? null);
  const [remember, setRemember] = useState(false);
  const current = list.find((e) => e.path === chosen) ?? list[0] ?? null;

  const pickOther = async () => {
    try {
      const p = await api.pickProgram();
      if (!p) return;
      setExtra((xs) => [p, ...xs.filter((x) => x.path !== p.path)]);
      setChosen(p.path);
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const submit = () => {
    if (!current) return;
    if (remember) setSettings({ editor: current });
    onClose();
    onPick(current);
  };

  return (
    <Modal
      title={t.files.editor.title}
      width={420}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Checkbox checked={remember} onChange={setRemember}>{t.files.editor.remember}</Checkbox>
          <span className="grow" />
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" disabled={!current}>{t.files.editor.open}</Button>
        </>
      }
    >
      <div className="editor-list" role="listbox">
        {list.map((e) => (
          <button
            key={e.path}
            type="button"
            role="option"
            aria-selected={current?.path === e.path}
            className={`editor-item ${current?.path === e.path ? "on" : ""}`}
            title={e.path}
            onClick={() => setChosen(e.path)}
            onDoubleClick={() => {
              setChosen(e.path);
              if (remember) setSettings({ editor: e });
              onClose();
              onPick(e);
            }}
          >
            <AppWindow size={15} />
            <span>{e.name}</span>
          </button>
        ))}
        {api.real && (
          <button type="button" className="editor-item other" onClick={() => void pickOther()}>
            <FolderSearch size={15} />
            <span>{t.files.editor.other}</span>
          </button>
        )}
      </div>
    </Modal>
  );
}
