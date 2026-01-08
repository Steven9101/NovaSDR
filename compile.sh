git pull

cargo build -p novasdr-server --release --features "soapysdr,clfft"

cd frontend

npm ci

npm run build 

cd ..

exit
