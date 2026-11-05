// The Google connection on the Settings page. Each user makes their own
// Google registration (a "Desktop app" client) and pastes it here. One
// connection serves all Google features: the calendar and Gmail now, Docs later.
import { useEffect, useRef, useState } from "react";
import * as api from "./api";

const BTN = "rounded-lg border border-stone-300 px-3 py-1.5 text-sm hover:bg-stone-200 disabled:opacity-40 dark:border-stone-700 dark:hover:bg-stone-800";
const PRI = "rounded-lg bg-[#252e4a] px-4 py-2 text-sm font-medium text-stone-50 hover:bg-[#323d62] disabled:opacity-40";
const INPUT = "w-full max-w-md rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm dark:border-stone-700 dark:bg-stone-900";

/** How to make the registration in Google Cloud Console. */
function Steps() {
  return (
    <ol className="list-decimal space-y-1 pl-5 text-sm text-stone-500">
      <li>
        Open <a href="https://console.cloud.google.com/projectcreate" className="underline">Google Cloud Console</a> and make a project (or pick one).
      </li>
      <li>
        Turn on the <a href="https://console.cloud.google.com/apis/library/calendar-json.googleapis.com" className="underline">Google Calendar API</a> and the{" "}
        <a href="https://console.cloud.google.com/apis/library/gmail.googleapis.com" className="underline">Gmail API</a> in that project.
      </li>
      <li>
        Open <a href="https://console.cloud.google.com/auth/overview" className="underline">Google Auth Platform</a> and set it up. For Audience, pick Internal when your account is in a company Google Workspace. Else pick External, and then click Publish app: in Testing, Google ends the login every 7 days. Google then shows "Google hasn't verified this app" at the login: this is your own registration, so click Advanced and continue.
      </li>
      <li>
        Under <a href="https://console.cloud.google.com/auth/clients" className="underline">Clients</a>, create a client of the type Desktop app. Copy its client ID and client secret into the boxes below.
      </li>
    </ol>
  );
}

export function GoogleSection(props: { onChanged: () => void }) {
  const [st, setSt] = useState<api.GoogleStatus | null>(null);
  const [id, setId] = useState("");
  const [secret, setSecret] = useState("");
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState<"" | "save" | "login" | "off" | "feature">("");
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);

  useEffect(() => {
    api.googleStatus().then(setSt).catch((e) => setMessage({ text: String(e), error: true }));
  }, []);

  async function run(kind: "save" | "login" | "off" | "feature", p: () => Promise<api.GoogleStatus>, done: (s: api.GoogleStatus) => string) {
    setBusy(kind);
    setMessage(null);
    try {
      const s = await p();
      setSt(s);
      setMessage({ text: done(s), error: false });
      props.onChanged();
      return true;
    } catch (e) {
      setMessage({ text: String(e), error: true });
      return false;
    } finally {
      setBusy("");
    }
  }

  const hasClient = !!st?.client_id && st.secret_set;
  // A feature that the user turned off is not missing.
  const notAllowed = (s: api.GoogleStatus) => s.features.filter(([n, on]) => !on && !s.off.includes(n)).map(([n]) => n);
  const missing = st?.connected ? notAllowed(st) : [];
  const allowed = (n: string) => !!st?.features.find(([f, on]) => f === n && on);
  function setFeature(name: string, on: boolean) {
    return run(
      "feature",
      () => api.setGoogleFeature(name, on),
      (s) => (!on ? `${name} is off. AIfred removed the saved data of ${name}.` : s.connected && !s.features.find(([f, a]) => f === name && a) ? `${name} is on. Connect again, and allow it.` : `${name} is on.`),
    );
  }
  // A second click starts a new login and stops the first one (for example
  // when the user closed the browser tab). Only the newest login shows its result.
  const loginCount = useRef(0);
  async function login() {
    const n = ++loginCount.current;
    const newest = () => n === loginCount.current;
    setBusy("login");
    setMessage(null);
    try {
      const s = await api.connectGoogle();
      if (!newest()) return;
      setSt(s);
      const off = notAllowed(s);
      setMessage({
        text: off.length ? `Connected, but Google did not allow: ${off.join(", ")}. Connect again and tick all boxes.` : `Connected as ${s.email ?? "your account"}.`,
        error: false,
      });
      props.onChanged();
    } catch (e) {
      if (newest()) setMessage({ text: String(e), error: true });
    } finally {
      if (newest()) setBusy("");
    }
  }
  // The login button stays on while a login waits, so the user can start again.
  const loginOff = !!busy && busy !== "login";
  // Disconnect also stays on while a login waits: the backend stops that
  // login. The stopped login is then not the newest, so its error does not
  // replace the message of the disconnect.
  function disconnect() {
    loginCount.current++;
    return run("off", api.disconnectGoogle, () => "Disconnected. Google forgot AIfred's access, and AIfred removed the key.");
  }

  return (
    <section className="mt-8 space-y-2" aria-label="Google">
      <h2 className="text-sm font-semibold">Google</h2>
      <p className="text-sm text-stone-500">
        AIfred reads your Google Calendar, so the day plan goes around your meetings. It reads your Gmail, so it finds the emails that wait for your answer and the emails where you wait for an answer. It reads only the sender, the subject, and Google's short preview of each email. It cannot send, change, or delete email, and it cannot change the calendar. Later Google features, like Docs, use the same connection.
      </p>
      {/* Off during a login too: the login already asked Google for the
          features, and a change now would not be in it. */}
      {st?.optional.map((n) => (
        <div key={n} className="text-sm">
          <label className="inline-flex items-center gap-2">
            <input type="checkbox" checked={!st.off.includes(n)} disabled={!!busy} onChange={(e) => setFeature(n, e.target.checked)} />
            Use {n}
          </label>
          {st.off.includes(n) && st.connected && allowed(n) && (
            <span className="ml-2 text-stone-500">Google still gives AIfred this access. Connect again to remove it.</span>
          )}
        </div>
      ))}
      {st?.connected ? (
        <>
          <p className="text-sm">
            Connected as <b>{st.email ?? "your Google account"}</b>.{" "}
            {st.features.map(([n, on]) => (
              <span key={n} className="mr-2">
                {n}: {st.off.includes(n) ? "turned off" : on ? "allowed" : "not allowed"}
              </span>
            ))}
          </p>
          {missing.length > 0 && <p className="text-sm text-amber-700">Google needs more access for {missing.join(", ")}. Connect again, and allow it.</p>}
          <div className="flex items-center gap-2">
            <button className={BTN} disabled={loginOff} onClick={login}>
              {busy === "login" ? "Waiting for the browser… (click to start again)" : "Connect again"}
            </button>
            <button className={BTN} disabled={loginOff} onClick={disconnect}>
              Disconnect
            </button>
          </div>
        </>
      ) : hasClient && !editing ? (
        <>
          <p className="text-sm text-stone-500">
            Your registration: <span className="break-all">{st!.client_id}</span>{" "}
            <button className="underline" onClick={() => (setEditing(true), setId(st!.client_id))}>
              Change
            </button>
          </p>
          <button className={PRI} disabled={loginOff} onClick={login}>
            {busy === "login" ? "Finish the login in your browser…" : "Connect Google"}
          </button>
          {busy === "login" && <p className="text-sm text-stone-500">Your browser shows Google's login page. If you closed it, click Connect Google again.</p>}
        </>
      ) : (
        <>
          <Steps />
          <label className="block text-sm">
            Client ID
            <input className={INPUT} value={id} onChange={(e) => setId(e.target.value)} placeholder="….apps.googleusercontent.com" autoComplete="off" />
          </label>
          <label className="block text-sm">
            Client secret
            <input className={INPUT} type="password" value={secret} onChange={(e) => setSecret(e.target.value)} autoComplete="off" />
          </label>
          <div className="flex items-center gap-2">
            <button
              className={PRI}
              disabled={!!busy || !id.trim() || !secret.trim()}
              onClick={async () => {
                if (await run("save", () => api.setGoogleClient(id, secret), () => "Saved. Now click Connect Google.")) {
                  setSecret("");
                  setEditing(false);
                }
              }}
            >
              Save registration
            </button>
            {editing && (
              <button className={BTN} onClick={() => setEditing(false)}>
                Cancel
              </button>
            )}
          </div>
        </>
      )}
      {message && (
        <p role="status" className={`text-sm ${message.error ? "text-red-600" : "text-stone-500"}`}>
          {message.text}
        </p>
      )}
    </section>
  );
}
