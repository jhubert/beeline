# OAuth client configuration for local development.
#
#   1. cp config.example.sh config.sh   (config.sh is gitignored)
#   2. fill in the values from your Google Cloud + Entra registrations
#   3. source config.sh    (before running `mailagent add-account` / `serve` / `mcp`)
#
# Client IDs are public; the Google desktop "client secret" is distributed with
# installed apps and is NOT truly confidential — but we keep all of it out of
# version control regardless.

export MAILAGENT_GOOGLE_CLIENT_ID="xxxxxxxx.apps.googleusercontent.com"
export MAILAGENT_GOOGLE_CLIENT_SECRET="GOCSPX-xxxxxxxx"
export MAILAGENT_MICROSOFT_CLIENT_ID="00000000-0000-0000-0000-000000000000"
