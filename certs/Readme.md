To use https:// for your WebSDR make the follow command on the certs folder
good is when you install on Debian/Ubuntu the follow Package sudo apt-get install openssl libssl-dev

openssl req -x509 -newkey rsa:2048 -nodes -keyout certs/privkey.pem -out certs/fullchain.pem -days 365 -subj "/CN=localhost"
