;; Repairs a Renovate pull request whose build failed, once the update has cooled.

(local declare
  {:name :build-failures
   :on :github.check_suite.completed
   :intent "Repair a Renovate pull request whose build failed, after its cooldown."
   :may [:attest :await :ask :push]
   :runs-per-hour 20
   :agents {:fixer {:prompt :fixer :vars [:repo]}}})

;; Repairs tried on one pull request before a person is asked.
(local attempt-cap 3)

;; The update has cooled and its build fails, so hold the commit, read the failed jobs' logs, and
;; ask for a repair. A repair that takes back the update would pass for the wrong reason.
(fn repair [github run fixer repo branch pull]
  (let [head pull.head
        number pull.number]
    (run.hold repo head)
    (let [logs (. (github.logs repo head) :text)
          {: patch : diff : said} (fixer.repair {: repo}
                                                {: logs "$checkout" {: repo :sha head}})]
      (if (. (github.undoes repo number patch) :undoes)
          {:kind :undone : repo : number : head : diff}
          {:kind :repaired : repo : number : branch : head : patch : diff : said}))))

;; What GitHub says now decides, since a payload is only a hint that something changed.
(fn gather [{: github : ages : run : fixer} event]
  (let [repo event.repo
        branch event.head_branch
        pull (if branch (github.pull repo branch) {:found false})]
    (if (or (not pull.found) pull.conflicted pull.running)
        {:kind :nothing}
        (not pull.failed)
        (if pull.repair_passed
            {:kind :passed : repo :head pull.head}
            {:kind :nothing})
        (let [attempts (run.attempts)
              number pull.number]
          (if (>= attempts.used attempt-cap)
              {:kind :exhausted : repo : number :head pull.head :asked attempts.asked}
              (case (. (ages.cooled pull.text) :age)
                :passed (repair github run fixer repo branch pull)
                :young {:kind :cooling}
                _ {:kind :undated : repo : number :asked attempts.asked}))))))

;; Fails the trust check and asks a person, with `body` for them to start from.
(fn stop-and-ask [{: attest : ask} repo number head question body]
  [(attest {:context "almanac/trust" :state :failure : repo :sha head})
   (ask {: question :target repo :pull number : body})])

;; Where a run ended up, turned into effects.
(fn policy [facts {: attest : await : ask : push}]
  (let [effects {: attest : ask}]
    (case facts
      {:kind :nothing} []
      {:kind :cooling} [(await "the update's cooldown to pass")]
      {:kind :undated :asked true} [(await "a person to look at this update")]
      {:kind :undated : repo : number}
      [(ask {:question "This update's build failed, and nothing says when it was published"
             :target repo
             :pull number
             :body "No agent reads an update before its cooldown passes, and this one has no publish time to measure from. It needs a person."})]
      {:kind :passed : repo : head}
      [(attest {:context "almanac/trust" :state :success : repo :sha head})]
      {:kind :exhausted :asked true} []
      {:kind :exhausted : repo : number : head}
      (stop-and-ask effects repo number head "Repairs did not fix this build" "")
      {:kind :undone : repo : number : head : diff}
      (stop-and-ask effects repo number head "A repair took back the update" diff)
      {:kind :repaired : repo : number : branch : head : patch : diff : said}
      (case (push {: repo
                   :pull number
                   : branch
                   :base head
                   :message "fix(deps): repair the build after this update"
                   :hold "almanac/trust"
                   : patch})
        (nil :empty) (stop-and-ask effects repo number head
                                   "A repair ended with no change to push" said)
        (nil _) (stop-and-ask effects repo number head
                              "A repair was refused by the patch policy" diff)
        pushed [pushed]))))

{: declare : gather : policy}
