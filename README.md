# Himalaya-Cache

Enabling caching ability for [himalaya](https://github.com/pimalaya/himalaya) email client.

The himalaya email client sends a request for every command run. Say you are doing this:

``` shell
himalaya folder list
himalaya envelope list --folder <folder>
himalaya message read --folder <folder> <id>
```

That can take an awful lot of time... and we know it needs not be that way because we can just cache the messages locally.

This is where `himalaya-cache` comes into place. You run `himalaya-cache sync` once and it will sync all the messages from the accounts you have configured in your `~/.config/himalaya/config.toml`. After then, you can run:

``` shell
himalaya-cache folder list
himalaya-cache envelope list --folder <folder>
himalaya-cache message read --folder <folder> <id>
```

Several things to be noted:

- this thing is 100% vibe coded because I do not know much about rust
- the reason why I develop this is because I use himalaya with [himalaya-emacs](https://github.com/dantecatalfamo/himalaya-emacs/), and:
  - I mostly just use emacs to read emails without managing them, which is why for now only subcommands associated with email reading is implemented
  - any subcommands not implemented here is forwarded instead to himalaya
- there is a lot of hard coding involved because I need an MVP here
- the code does not handle cli flags that well
  - if you place flags before the subcommand, like `himalaya-cache -o json message read`, it is not recognized
  - if you want to use this with himalaya-emacs, you should modify the functions in `himalaya-process` that runs `himalaya-cache` and move the `-c` / `-o` flags to the end (you can do this via `advice-add`)
  
Any contribution is most welcome here.
