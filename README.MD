# GPT tool_calls Demo

This is a naive demo for using gpt to make tool_calls on flows. It lives on a slack channel where you choose to host it.

It has 3 built-in functions:
1. Get the current weather of a city by querying openweathermap.org
2. Get the text content of a webpage with the webpage-scraper tool by the flows library
3. Get the time of day by running code "locally"

Use "tool_calls" as a command to run the bot. Here's an example:
```
tool_calls I'd like to know the time of the day
```

Your chat with gpt on slack will stop once it reaches one of the 3 designated functions, otherwise it continues. Try it out!