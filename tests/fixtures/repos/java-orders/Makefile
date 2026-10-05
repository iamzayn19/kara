.PHONY: test clean
test:
	rm -rf out && mkdir -p out
	javac -d out src/orders/*.java test/orders/*.java
	java -ea -cp out orders.TestRunner
clean:
	rm -rf out
