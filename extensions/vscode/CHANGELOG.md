# Changelog

## Unreleased

- Published as `iamzayn19.kara-agent` ("Kara Agent").
- Uses the Kara diamond mark for the marketplace and activity-bar icons.
- Windows: no console window flashes up for `kara serve` or `kara connect`.
- Up/Down in the chat box recall earlier prompts.
- A model download can be cancelled from its progress notification; it resumes next time.
- "Connect to a Kara machine" passes the token to `kara connect` through the
  environment instead of the command line, where other local users could see it.

## 0.1.0

- First version: chat sidebar, streamed agent events, diff previews with
  per-file reject, permission dialogs, model selection, doctor, selection and
  diagnostics context, plan (`/morpheus`) and review (`/oracle`) modes.
