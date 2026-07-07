data {
symbols = ["AAPL",   "MSFT"]
period="1y"
    interval =    "1d"
source = "yahoo"
}

strategy Messy {
on bar {
x=close+open
if x>0 {
OPEN(symbol,100.0)
}
}
}
